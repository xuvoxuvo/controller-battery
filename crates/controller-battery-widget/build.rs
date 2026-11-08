//! Build script: embeds the executable icon and the VERSIONINFO resource.
//!
//! The same artwork as the tray icon is rendered from `src/icon_glyph.rs` into
//! a multi-size `.ico`, described by a small `.rc`, and compiled with whichever
//! resource compiler this toolchain has: `rc.exe` for MSVC, `windres` for GNU.
//!
//! A missing resource compiler is a warning, not an error — the widget still
//! builds, just without version info and a custom icon.

#![allow(dead_code)]
#![allow(clippy::needless_range_loop)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The same artwork the tray icon uses, so the exe icon never drifts.
mod glyph {
    include!("src/icon_glyph.rs");
}
use glyph::{icon_pixels, OUTLINE_NEUTRAL};

/// Icon sizes written into the `.ico` (Explorer picks the closest one).
const ICON_SIZES: [u32; 4] = [16, 32, 48, 256];
/// Fill level drawn into the static executable icon.
const ICON_PERCENT: u8 = 80;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon_glyph.rs");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let Ok(out_dir) = env::var("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    let ico = out_dir.join("controller-battery.ico");
    let rc = out_dir.join("controller-battery.rc");

    if let Err(error) = write_ico(&ico).and_then(|()| write_rc(&rc, &ico)) {
        println!("cargo:warning=could not prepare the icon resource: {error}");
        return;
    }

    let compiled = match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => compile_with_rc(&rc, &out_dir.join("controller-battery.res")),
        Ok("gnu") => compile_with_windres(&rc, &out_dir.join("controller-battery-res.o")),
        _ => None,
    };

    match compiled {
        Some(path) => println!("cargo:rustc-link-arg-bins={}", path.display()),
        None => println!(
            "cargo:warning=no resource compiler found (rc.exe / windres); \
             the executable gets no icon and no version info"
        ),
    }
}

/// Multi-size 32 bpp `.ico` with an alpha channel.
fn write_ico(path: &Path) -> std::io::Result<()> {
    let images: Vec<(u32, Vec<u8>)> = ICON_SIZES
        .iter()
        .map(|&size| {
            let pixels = icon_pixels(size as usize, Some(ICON_PERCENT), false, OUTLINE_NEUTRAL);
            (size, pixels)
        })
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());

    let mut offset = 6 + 16 * images.len();
    let mut blobs = Vec::with_capacity(images.len());
    for (size, pixels) in &images {
        let blob = dib(*size as usize, pixels);
        let dimension = if *size >= 256 { 0 } else { *size as u8 };
        out.push(dimension);
        out.push(dimension);
        out.push(0); // palette entries
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += blob.len();
        blobs.push(blob);
    }
    for blob in blobs {
        out.extend_from_slice(&blob);
    }

    fs::write(path, out)
}

/// One `.ico` image: `BITMAPINFOHEADER`, bottom-up BGRA pixels, empty AND mask.
fn dib(size: usize, bgra: &[u8]) -> Vec<u8> {
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + size * size * 4 + mask_row * size);

    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(size as i32).to_le_bytes()); // biWidth
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // XOR + AND height
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((size * size * 4) as u32).to_le_bytes()); // biSizeImage
    out.extend_from_slice(&[0u8; 16]); // resolution, palette counts

    for row in (0..size).rev() {
        out.extend_from_slice(&bgra[row * size * 4..(row + 1) * size * 4]);
    }
    out.resize(out.len() + mask_row * size, 0);
    out
}

/// `.rc` with the icon and a VERSIONINFO block. ASCII only, so neither
/// compiler has to guess a code page.
fn write_rc(path: &Path, ico: &Path) -> std::io::Result<()> {
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_owned());
    let [major, minor, patch, build] = version_quad(&version);
    let icon = ico.display().to_string().replace('\\', "/");

    let rc = format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
 FILEVERSION {major},{minor},{patch},{build}
 PRODUCTVERSION {major},{minor},{patch},{build}
 FILEFLAGSMASK 0x3fL
 FILEFLAGS 0x0L
 FILEOS 0x40004L
 FILETYPE 0x1L
 FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "xuvoxuvo"
            VALUE "FileDescription", "Controller Battery - game controller battery widget"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "controller-battery"
            VALUE "LegalCopyright", "MIT License"
            VALUE "OriginalFilename", "controller-battery.exe"
            VALUE "ProductName", "Controller Battery"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );

    fs::write(path, rc)
}

fn version_quad(version: &str) -> [u16; 4] {
    let mut parts = [0u16; 4];
    for (slot, part) in parts.iter_mut().zip(version.split(['.', '-', '+'])) {
        *slot = part.parse().unwrap_or(0);
    }
    parts
}

fn compile_with_rc(rc: &Path, res: &Path) -> Option<PathBuf> {
    let compiler = find_in_path("rc.exe").or_else(find_windows_sdk_rc)?;
    let output = Command::new(&compiler)
        .arg("/nologo")
        .arg(format!("/fo{}", res.display()))
        .arg(rc)
        .output()
        .ok()?;
    if !output.status.success() {
        println!(
            "cargo:warning=rc.exe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    Some(res.to_path_buf())
}

fn compile_with_windres(rc: &Path, object: &Path) -> Option<PathBuf> {
    let compiler = find_in_path("windres.exe").or_else(|| find_in_path("windres"))?;
    let output = Command::new(&compiler)
        .arg("--output-format=coff")
        .arg("-i")
        .arg(rc)
        .arg("-o")
        .arg(object)
        .output()
        .ok()?;
    if !output.status.success() {
        println!(
            "cargo:warning=windres failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    Some(object.to_path_buf())
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    for dir in env::split_paths(&env::var_os("PATH")?) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// `C:\Program Files (x86)\Windows Kits\10\bin\<version>\x64\rc.exe`.
fn find_windows_sdk_rc() -> Option<PathBuf> {
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(base) = env::var_os(key) else {
            continue;
        };
        let root = PathBuf::from(base)
            .join("Windows Kits")
            .join("10")
            .join("bin");
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };

        let mut versions: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        versions.sort();
        versions.reverse();
        for version in versions {
            for arch in ["x64", "x86", "arm64"] {
                let candidate = version.join(arch).join("rc.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
