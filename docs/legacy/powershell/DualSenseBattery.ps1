<#
    DualSense Battery Widget
    读取 PS5 DualSense 手柄的 HID 输入报告，在桌面角落显示电量。
    无需安装任何驱动/运行时（纯 PowerShell + 系统 HID 接口）。

    用法:
      DualSenseBattery.ps1              # 启动悬浮窗
      DualSenseBattery.ps1 -Once        # 只在控制台输出一次电量
      DualSenseBattery.ps1 -InstallStartup / -RemoveStartup
      DualSenseBattery.ps1 -Diag        # 打印 DPI / 屏幕信息，排查缩放问题
#>
param(
    [switch]$Once,
    [switch]$InstallStartup,
    [switch]$RemoveStartup,
    [switch]$Diag
)

$ErrorActionPreference = 'Stop'

# DPI 感知必须在任何 WinForms 初始化之前设置，否则窗口会被系统放大、坐标错位
if (-not ('DpiHelper' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class DpiHelper {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    public static bool Enable() {
        try { return SetProcessDPIAware(); } catch { return false; }
    }
}
'@
}
$script:DpiOk = [DpiHelper]::Enable()

$script:ScriptPath = $PSCommandPath
$script:SettingsDir = Join-Path $env:LOCALAPPDATA 'DualSenseBattery'
$script:StartupLnk = Join-Path ([Environment]::GetFolderPath('Startup')) 'DualSense 电量.lnk'

# ---------------------------------------------------------------- HID 读取

$cs = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;

public class Reading
{
    public bool Ok;
    public int Percent;
    public int Charging;      // 0=放电 1=充电 2=充满 3=异常 4=未知
    public bool IsBluetooth;
    public bool IsCompat;     // 蓝牙简化报文：电池/充电字段可能滞后，只能当近似值
    public uint Sig;          // 报文指纹，用来判断数据是否还在刷新
    public string Error = "";
}

public static class DS
{
    [StructLayout(LayoutKind.Sequential)]
    struct SP_DEVICE_INTERFACE_DATA
    {
        public int cbSize;
        public Guid InterfaceClassGuid;
        public int Flags;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct HIDD_ATTRIBUTES
    {
        public int Size;
        public ushort VendorID;
        public ushort ProductID;
        public ushort VersionNumber;
    }

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr SetupDiGetClassDevs(ref Guid ClassGuid, IntPtr Enumerator, IntPtr hwndParent, int Flags);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SetupDiEnumDeviceInterfaces(IntPtr DeviceInfoSet, IntPtr DeviceInfoData, ref Guid InterfaceClassGuid, int MemberIndex, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr DeviceInfoSet, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData, IntPtr DeviceInterfaceDetailData, int DeviceInterfaceDetailDataSize, out int RequiredSize, IntPtr DeviceInfoData);

    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiDestroyDeviceInfoList(IntPtr DeviceInfoSet);

    [DllImport("hid.dll", SetLastError = true)]
    static extern bool HidD_GetAttributes(IntPtr HidDeviceObject, ref HIDD_ATTRIBUTES Attributes);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateFile(string fileName, uint desiredAccess, uint shareMode, IntPtr securityAttributes, uint creationDisposition, uint flagsAndAttributes, IntPtr templateFile);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr hObject);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool ReadFile(IntPtr hFile, byte[] lpBuffer, int nNumberOfBytesToRead, out int lpNumberOfBytesRead, IntPtr lpOverlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CancelIo(IntPtr hFile);

    const uint GENERIC_READ = 0x80000000;
    const uint FILE_SHARE_READ = 0x00000001;
    const uint FILE_SHARE_WRITE = 0x00000002;
    const uint OPEN_EXISTING = 3;

    // DualSense: PID 0CE6；DualSense Edge: PID 0DF2
    static bool IsSupported(ushort pid)
    {
        return pid == 0x0CE6 || pid == 0x0DF2;
    }

    // 列出所有 DualSense 的 HID 接口（USB 与蓝牙可能同时存在）
    public static List<string> Paths(out List<bool> bt)
    {
        bt = new List<bool>();
        var found = new List<string>();
        Guid hidGuid = new Guid("4D1E55B2-F16F-11CF-88CB-001111000030");
        IntPtr set = SetupDiGetClassDevs(ref hidGuid, IntPtr.Zero, IntPtr.Zero, 0x12);
        if (set == IntPtr.Zero || set == (IntPtr)(-1)) return found;

        try
        {
            var did = new SP_DEVICE_INTERFACE_DATA();
            did.cbSize = Marshal.SizeOf(typeof(SP_DEVICE_INTERFACE_DATA));
            for (int i = 0; SetupDiEnumDeviceInterfaces(set, IntPtr.Zero, ref hidGuid, i, ref did); i++)
            {
                int required;
                SetupDiGetDeviceInterfaceDetail(set, ref did, IntPtr.Zero, 0, out required, IntPtr.Zero);
                if (required <= 0) continue;

                IntPtr detail = Marshal.AllocHGlobal(required);
                try
                {
                    Marshal.WriteInt32(detail, IntPtr.Size == 8 ? 8 : 6);
                    if (!SetupDiGetDeviceInterfaceDetail(set, ref did, detail, required, out required, IntPtr.Zero)) continue;
                    string path = Marshal.PtrToStringUni(new IntPtr(detail.ToInt64() + 4));
                    if (string.IsNullOrEmpty(path)) continue;

                    IntPtr h = CreateFile(path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero);
                    if (h == (IntPtr)(-1)) continue;
                    try
                    {
                        var attr = new HIDD_ATTRIBUTES();
                        attr.Size = Marshal.SizeOf(typeof(HIDD_ATTRIBUTES));
                        if (!HidD_GetAttributes(h, ref attr)) continue;
                        if (attr.VendorID != 0x054C || !IsSupported(attr.ProductID)) continue;

                        found.Add(path);
                        bt.Add(path.IndexOf("00001124", StringComparison.OrdinalIgnoreCase) >= 0);
                    }
                    finally { CloseHandle(h); }
                }
                finally { Marshal.FreeHGlobal(detail); }
            }
        }
        finally { SetupDiDestroyDeviceInfoList(set); }
        return found;
    }

    // 优先 USB（实时、字段完整），否则用蓝牙
    public static string Find(out bool bt)
    {
        List<bool> flags;
        List<string> paths = Paths(out flags);
        for (int i = 0; i < paths.Count; i++)
        {
            if (!flags[i]) { bt = false; return paths[i]; }
        }
        if (paths.Count > 0) { bt = true; return paths[0]; }
        bt = false;
        return null;
    }

    static bool Plausible(byte[] d, int off)
    {
        if (off < 0 || d.Length < off + 53) return false;
        int st = d[off + 52];
        int cap = st & 0x0F;
        int chg = (st >> 4) & 0x0F;
        if (cap > 10) return false;
        return chg <= 2 || chg == 0xA || chg == 0xB || chg == 0xF;
    }

    public static bool Decode(byte[] buf, int len, bool bt, out Reading r, out string err)
    {
        r = new Reading();
        err = null;

        uint sig = 2166136261u;
        for (int i = 0; i < len; i++) { sig ^= buf[i]; sig *= 16777619u; }
        r.Sig = sig;
        r.IsBluetooth = bt;

        // 完整报文：USB 64 字节 0x01；蓝牙 78 字节 0x31（部分固件发 78 字节 0x01）
        int off = -1;
        if (len == 64 && buf[0] == 0x01) off = 1;
        else if (len >= 78) { if (Plausible(buf, 2)) off = 2; else if (Plausible(buf, 1)) off = 1; }

        // 78 字节的 0x01 是蓝牙简化报文：结构一致，但电池/充电字段刷新很慢
        r.IsCompat = (len >= 78 && buf[0] == 0x01);

        if (off < 0 || len < off + 53)
        {
            err = "未识别的报文格式";
            return false;
        }

        int status = buf[off + 52];
        int cap = status & 0x0F;
        int chg = (status >> 4) & 0x0F;

        r.Ok = true;
        switch (chg)
        {
            case 0x0: r.Percent = Math.Min(cap * 10 + 5, 100); r.Charging = 0; break;
            case 0x1: r.Percent = Math.Min(cap * 10 + 5, 100); r.Charging = 1; break;
            case 0x2: r.Percent = 100; r.Charging = 2; break;
            case 0xA:
            case 0xB: r.Percent = 0; r.Charging = 3; break;
            default:  r.Percent = 0; r.Charging = 4; break;
        }
        return true;
    }

    static bool ReadReport(IntPtr h, byte[] buf, int timeoutMs, out int read, out string err)
    {
        read = 0;
        err = null;
        int errCode = 0;
        string inner = null;
        int localRead = 0;
        var done = new ManualResetEvent(false);

        Thread t = new Thread(delegate()
        {
            try
            {
                if (!ReadFile(h, buf, buf.Length, out localRead, IntPtr.Zero)) errCode = Marshal.GetLastWin32Error();
            }
            catch (Exception ex) { inner = ex.Message; }
            finally { done.Set(); }
        });
        t.IsBackground = true;
        t.Start();

        if (!done.WaitOne(timeoutMs))
        {
            CancelIo(h);
            err = "读取超时";
            return false;
        }
        read = localRead;
        if (inner != null) { err = inner; return false; }
        if (errCode != 0) { err = "读取失败 (" + errCode + ")"; return false; }
        return true;
    }

    public static IntPtr Open(string path, out string err)
    {
        err = null;
        IntPtr h = CreateFile(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero);
        if (h == (IntPtr)(-1))
        {
            int e = Marshal.GetLastWin32Error();
            err = (e == 5) ? "设备被其他程序占用" : ("无法打开设备 (" + e + ")");
            return IntPtr.Zero;
        }
        return h;
    }

    public static bool TryRead(IntPtr h, bool bt, int timeoutMs, out Reading r, out string err)
    {
        var buf = new byte[128];
        int read;
        string e;
        if (!ReadReport(h, buf, timeoutMs, out read, out e)) { r = null; err = e; return false; }
        if (read < 60) { r = null; err = "报文不完整"; return false; }
        return Decode(buf, read, bt, out r, out err);
    }

    // 单次读取（-Once 用）：找设备、开句柄、读一条
    public static Reading ReadOnce(int timeoutMs)
    {
        bool bt;
        string path = Find(out bt);
        if (path == null)
        {
            var none = new Reading();
            none.Error = "未检测到手柄";
            return none;
        }

        string err;
        IntPtr h = Open(path, out err);
        if (h == IntPtr.Zero)
        {
            var bad = new Reading();
            bad.Error = err;
            return bad;
        }

        try
        {
            Reading r;
            if (!TryRead(h, bt, timeoutMs, out r, out err))
            {
                var bad = new Reading();
                bad.Error = err;
                return bad;
            }
            return r;
        }
        finally { CloseHandle(h); }
    }
}

public static class DSMonitor
{
    // 0=未检测到设备 1=正常 2=休眠（数据不再刷新） 3=错误
    public static volatile int State;
    public static volatile int Percent;
    public static volatile int Charging;
    public static volatile bool IsBluetooth;
    public static volatile bool IsCompat;
    public static volatile string Error = "";

    static Thread worker;
    static volatile bool running;

    public static void Start()
    {
        if (worker != null) return;
        running = true;
        worker = new Thread(Loop);
        worker.IsBackground = true;
        worker.Start();
    }

    public static void Stop()
    {
        running = false;
        worker = null;
    }

    static void Loop()
    {
        IntPtr h = IntPtr.Zero;
        bool bt = false;
        uint lastSig = 0;
        bool hasSig = false;
        DateTime lastChange = DateTime.UtcNow;

        while (running)
        {
            if (h == IntPtr.Zero)
            {
                string path;
                try { path = DS.Find(out bt); }
                catch (Exception ex) { path = null; Error = ex.Message; }

                if (path == null)
                {
                    State = 0;
                    Error = "";
                    hasSig = false;
                    Sleep(1200);
                    continue;
                }

                string openErr;
                h = DS.Open(path, out openErr);
                if (h == IntPtr.Zero)
                {
                    State = 3;
                    Error = openErr;
                    Sleep(1500);
                    continue;
                }
                lastChange = DateTime.UtcNow;
            }

            Reading r;
            string err;
            if (!DS.TryRead(h, bt, 1000, out r, out err))
            {
                CloseHandleQuiet(h);
                h = IntPtr.Zero;
                State = 2;
                Error = err;
                Sleep(800);
                continue;
            }

            if (!hasSig || r.Sig != lastSig)
            {
                lastSig = r.Sig;
                hasSig = true;
                lastChange = DateTime.UtcNow;
            }

            double idle = (DateTime.UtcNow - lastChange).TotalSeconds;
            Percent = r.Percent;
            Charging = r.Charging;
            IsBluetooth = r.IsBluetooth;
            IsCompat = r.IsCompat;
            Error = "";

            if (idle > 5.0) { State = 2; Sleep(300); }
            else { State = 1; Sleep(200); }
        }

        CloseHandleQuiet(h);
    }

    static void Sleep(int ms)
    {
        for (int i = 0; i < ms / 100 && running; i++) Thread.Sleep(100);
        int rest = ms % 100;
        if (rest > 0 && running) Thread.Sleep(rest);
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr hObject);

    static void CloseHandleQuiet(IntPtr h)
    {
        if (h != IntPtr.Zero) { try { CloseHandle(h); } catch { } }
    }
}
'@

Add-Type -TypeDefinition $cs -Language CSharp

# ---------------------------------------------------------------- 启动项

function Test-Startup {
    Test-Path $script:StartupLnk
}

function Set-Startup([bool]$enable) {
    if (-not $enable) {
        Remove-Item $script:StartupLnk -Force -ErrorAction SilentlyContinue
        return
    }
    $ws = New-Object -ComObject WScript.Shell
    $sc = $ws.CreateShortcut($script:StartupLnk)
    $sc.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $sc.Arguments = "-NoProfile -STA -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$script:ScriptPath`""
    $sc.WorkingDirectory = Split-Path $script:ScriptPath -Parent
    $sc.Description = 'DualSense 电量浮窗'
    $sc.Save()
}

if ($InstallStartup) { Set-Startup $true; Write-Host '已设置开机自动启动。'; exit 0 }
if ($RemoveStartup) { Set-Startup $false; Write-Host '已取消开机自动启动。'; exit 0 }

# ---------------------------------------------------------------- 状态文本

function Get-StateText($info) {
    switch ($info.Charging) {
        0 { '使用中' }
        1 { '充电中' }
        2 { '已充满' }
        3 { '电池异常' }
        default { '状态未知' }
    }
}

# ---------------------------------------------------------------- 单次模式

if ($Once) {
    $b = [DS]::ReadOnce(2500)
    if (-not $b.Ok) {
        Write-Host "DualSense: $($b.Error)"
        Write-Host '  请确认：手柄已通过蓝牙配对或插好 USB 数据线，然后按 PS 键唤醒。'
        exit 1
    }
    $link = if ($b.IsBluetooth) { '蓝牙' } else { 'USB' }
    if ($b.IsCompat) {
        Write-Host ("DualSense {0} · 电量约 {1}% · 充电状态不可用（手柄在发简化报文）" -f $link, $b.Percent)
        Write-Host '  简化报文里的电池字段不刷新，插一下 USB 数据线可拿到实时值。'
    }
    else {
        Write-Host ("DualSense {0} · 电量 {1}% · {2}" -f $link, $b.Percent, (Get-StateText $b))
    }

    Start-Sleep -Milliseconds 350
    $b2 = [DS]::ReadOnce(1500)
    if ($b2.Ok -and $b2.Sig -eq $b.Sig) {
        Write-Host '  注意：报文没有刷新（手柄空闲时会停发），读数可能是空闲前的旧值；动一下手柄再看。'
    }
    exit 0
}

# ---------------------------------------------------------------- 单实例

$mutex = New-Object System.Threading.Mutex($false, 'Local\DualSenseBatteryWidget')
if (-not $mutex.WaitOne(0)) {
    Write-Host '浮窗已在运行。'
    exit 0
}

# ---------------------------------------------------------------- 界面

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()

$probeBmp = New-Object System.Drawing.Bitmap(1, 1)
$probeG = [System.Drawing.Graphics]::FromImage($probeBmp)
$script:DpiX = $probeG.DpiX
$script:Scale = $script:DpiX / 96.0
$probeG.Dispose(); $probeBmp.Dispose()

$script:W = [int](200 * $script:Scale)
$script:H = [int](76 * $script:Scale)
$script:R = [int](16 * $script:Scale)

if ($Diag) {
    Write-Host "DpiOk=$script:DpiOk DpiX=$script:DpiX Scale=$script:Scale"
    Write-Host "WorkingArea=$([System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea)"
    Write-Host "Window=$script:W x $script:H"
    exit 0
}

function New-RoundedPath([int]$w, [int]$h, [int]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $p.AddArc(0, 0, $d, $d, 180, 90)
    $p.AddArc($w - $d - 1, 0, $d, $d, 270, 90)
    $p.AddArc($w - $d - 1, $h - $d - 1, $d, $d, 0, 90)
    $p.AddArc(0, $h - $d - 1, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

$form = New-Object System.Windows.Forms.Form
$form.FormBorderStyle = 'None'
$form.StartPosition = 'Manual'
$form.TopMost = $true
$form.ShowInTaskbar = $false
$form.MaximizeBox = $false
$form.MinimizeBox = $false
$form.AutoScaleMode = 'None'
$form.Text = 'DualSense 电量'
$form.BackColor = [System.Drawing.Color]::FromArgb(22, 22, 26)
$form.ClientSize = New-Object System.Drawing.Size($script:W, $script:H)
$form.Region = New-Object System.Drawing.Region((New-RoundedPath $script:W $script:H $script:R))

# 位置记忆
$script:PosFile = Join-Path $script:SettingsDir 'position.txt'
$startX = -1; $startY = -1
if (Test-Path $script:PosFile) {
    $parts = (Get-Content $script:PosFile -Raw).Trim() -split ','
    if ($parts.Count -eq 2) {
        $startX = [int]$parts[0]; $startY = [int]$parts[1]
    }
}
$wa = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
if ($startX -lt 0 -or $startY -lt 0 -or $startX -gt ($wa.Right - 40) -or $startY -gt ($wa.Bottom - 40)) {
    $startX = $wa.Right - $script:W - [int](24 * $script:Scale)
    $startY = $wa.Bottom - $script:H - [int](72 * $script:Scale)
}
$form.Location = New-Object System.Drawing.Point($startX, $startY)

# 画笔资源
$script:FontLabel = New-Object System.Drawing.Font('Segoe UI', 8.5)
$script:FontBig = New-Object System.Drawing.Font('Segoe UI', 23, [System.Drawing.FontStyle]::Bold)
$script:FontState = New-Object System.Drawing.Font('Microsoft YaHei UI', 10)
$script:BrushMuted = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(150, 156, 165))
$script:BrushTrack = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(58, 58, 66))
$script:BrushText = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(240, 242, 246))
$script:BrushAccent = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(52, 211, 153))
$script:PenBorder = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(46, 48, 54), 1)

$script:LastKey = ''
$script:State = 0
$script:Pct = 0
$script:Charging = 0
$script:Connected = $false
$script:Bt = $false
$script:Compat = $false
$script:Hint = ''

$form.Add_Paint({
    param($s, $e)
    $g = $e.Graphics
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::ClearTypeGridFit
    $g.Clear($form.BackColor)

    $w = $s.Width; $h = $s.Height
    $S = $script:Scale
    $g.DrawPath($script:PenBorder, (New-RoundedPath $w $h $script:R))

    $label = 'DUALSENSE'
    if ($script:State -eq 1 -or $script:State -eq 2) {
        $label += $(if ($script:Bt) { ' · 蓝牙' } else { ' · USB' })
    }
    $g.DrawString($label, $script:FontLabel, $script:BrushMuted, (14 * $S), (7 * $S))

    $accent = [System.Drawing.Color]::FromArgb(107, 114, 128)
    if ($script:State -eq 1) {
        switch ($script:Charging) {
            1 { $accent = [System.Drawing.Color]::FromArgb(56, 189, 248) }
            2 { $accent = [System.Drawing.Color]::FromArgb(52, 211, 153) }
            default {
                if ($script:Pct -ge 50) { $accent = [System.Drawing.Color]::FromArgb(52, 211, 153) }
                elseif ($script:Pct -ge 20) { $accent = [System.Drawing.Color]::FromArgb(251, 191, 36) }
                else { $accent = [System.Drawing.Color]::FromArgb(248, 113, 113) }
            }
        }
    }
    if ($null -eq $script:BrushAccent -or $script:BrushAccent.Color -ne $accent) {
        if ($script:BrushAccent) { $script:BrushAccent.Dispose() }
        $script:BrushAccent = New-Object System.Drawing.SolidBrush($accent)
    }

    if ($script:State -eq 1 -or $script:State -eq 2) {
        # 空闲时手柄不再发报文，显示的是最后一次读数，用灰色区分
        $isIdle = ($script:State -eq 2)
        # 简化报文（78 字节 0x01）里的电池/充电字段不刷新，只能当旧值看
        $isCompat = $script:Compat
        $pctBrush = if ($isIdle -or $isCompat) { $script:BrushMuted } else { $script:BrushText }
        $pctText = if ($isCompat) { "≈$($script:Pct)%" } else { "$($script:Pct)%" }
        $g.DrawString($pctText, $script:FontBig, $pctBrush, (12 * $S), (20 * $S))

        $state = if ($isIdle) {
            '空闲'
        }
        elseif ($isCompat) {
            '状态未知'
        }
        else {
            switch ($script:Charging) {
                0 { '使用中' }
                1 { '充电中' }
                2 { '已充满' }
                3 { '电池异常' }
                default { '状态未知' }
            }
        }
        $stateBrush = if ($isIdle -or $isCompat) { $script:BrushMuted } else { $script:BrushAccent }
        $size = $g.MeasureString($state, $script:FontState)
        $g.DrawString($state, $script:FontState, $stateBrush, ($w - $size.Width - (14 * $S)), (30 * $S))

        $barX = 14 * $S; $barW = $w - (28 * $S); $barY = $h - (16 * $S); $barH = 5 * $S
        $g.FillRectangle($script:BrushTrack, $barX, $barY, $barW, $barH)
        $fillW = [int]($barW * $script:Pct / 100.0)
        if ($fillW -gt 0) { $g.FillRectangle($script:BrushAccent, $barX, $barY, $fillW, $barH) }
    }
    else {
        $title = switch ($script:State) {
            3 { '读取失败' }
            default { '未检测到手柄' }
        }
        $hint = $script:Hint
        if (-not $hint) {
            $hint = switch ($script:State) {
                3 { '请重试或重插 USB 线' }
                default { '蓝牙配对或插 USB 线' }
            }
        }

        $g.DrawString('--', $script:FontBig, $script:BrushMuted, (12 * $S), (20 * $S))
        $g.DrawString($title, $script:FontState, $script:BrushMuted, (76 * $S), (18 * $S))
        $g.DrawString($hint, $script:FontLabel, $script:BrushMuted, (76 * $S), (44 * $S))
    }
})

# 拖拽
$script:Dragging = $false
$script:DragOffset = New-Object System.Drawing.Point(0, 0)

$form.Add_MouseDown({
    param($s, $e)
    if ($e.Button -eq [System.Windows.Forms.MouseButtons]::Left) {
        $script:Dragging = $true
        $script:DragOffset = $e.Location
    }
})
$form.Add_MouseMove({
    param($s, $e)
    if ($script:Dragging) {
        $p = [System.Windows.Forms.Cursor]::Position
        $form.Location = New-Object System.Drawing.Point(($p.X - $script:DragOffset.X), ($p.Y - $script:DragOffset.Y))
    }
})
$form.Add_MouseUp({
    param($s, $e)
    if ($script:Dragging) {
        $script:Dragging = $false
        try {
            if (-not (Test-Path $script:SettingsDir)) { New-Item -ItemType Directory -Path $script:SettingsDir -Force | Out-Null }
            "$($form.Left),$($form.Top)" | Set-Content -Path $script:PosFile -Encoding ASCII
        } catch { }
    }
})

# 右键菜单
$menu = New-Object System.Windows.Forms.ContextMenuStrip
$miRefresh = $menu.Items.Add('立即刷新')
$miStartup = $menu.Items.Add('开机自动启动')
$miStartup.CheckOnClick = $true
$miStartup.Checked = Test-Startup
$miExit = $menu.Items.Add('退出')

$miRefresh.Add_Click({
    $script:LastKey = ''
    $form.Invalidate()
})
$miStartup.Add_Click({
    try { Set-Startup ([bool]$miStartup.Checked) } catch { }
})
$miExit.Add_Click({
    $form.Close()
})
$form.ContextMenuStrip = $menu

# 定时刷新
[DSMonitor]::Start()
$script:Timer = New-Object System.Windows.Forms.Timer
$script:Timer.Interval = 400
$script:Timer.Add_Tick({
    $state = [DSMonitor]::State
    $pct = [DSMonitor]::Percent
    $chg = [DSMonitor]::Charging
    $bt = [DSMonitor]::IsBluetooth
    $compat = [DSMonitor]::IsCompat
    $err = [DSMonitor]::Error
    $key = "$state|$pct|$chg|$bt|$compat|$err"
    if ($key -ne $script:LastKey) {
        $script:LastKey = $key
        $script:State = $state
        $script:Pct = $pct
        $script:Charging = $chg
        $script:Bt = $bt
        $script:Compat = $compat
        $script:Connected = ($state -eq 1)

        $script:Hint = ''
        if ($state -eq 3) {
            if ($err -like '*占用*') { $script:Hint = '关闭占用程序或重插' }
            elseif ($err) { $script:Hint = ([string]$err) }
        }
        $form.Invalidate()
    }
})
$script:Timer.Start()

$form.Add_FormClosed({
    $script:Timer.Stop()
    [DSMonitor]::Stop()
})

[System.Windows.Forms.Application]::Run($form)
