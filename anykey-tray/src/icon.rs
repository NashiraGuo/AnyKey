//! 托盘图标 —— 加载预生成图片资源（include_bytes!，无运行时 PNG 解码依赖）：
//!   运行态：白色圆角方形底 + 黑色 ∀（任意符号）+ 底部黑色 homing bar（键帽基准凸起）；
//!   暂停态：整个图标反色（黑底 + 白色 ∀/homing bar）。
//!   资源来自 icon/anykey-app-icon-white-small-3.png 裁剪出的预览稿：
//!     icon/tray-preview-normal.png / tray-preview-paused.png（1024px）
//!   生成脚本把预览稿缩成 64x64 原始 RGBA 写入 assets/*.rgba，
//!   保证托盘图标与确认过的设计稿逐像素一致。

use std::ffi::c_void;
use std::mem;

use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

const ICON_SIZE: u32 = 64;

/// 运行态：白底黑墨（64x64 RGBA）
const NORMAL_RGBA: &[u8] = include_bytes!("../assets/tray-normal.rgba");
/// 暂停态：黑底白墨（64x64 RGBA）
const PAUSED_RGBA: &[u8] = include_bytes!("../assets/tray-paused.rgba");

/// 把 RGBA 资源写入 32bpp top-down DIB（little-endian BGRA，值 = 0xAABBGGRR）
fn write_icon_bits(bits: *mut c_void, size: u32, rgba: &[u8]) {
    assert_eq!(rgba.len(), (size * size * 4) as usize, "图标资源尺寸不符");
    let pixels =
        unsafe { std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize) };
    for (i, p) in pixels.iter_mut().enumerate() {
        let r = rgba[i * 4] as u32;
        let g = rgba[i * 4 + 1] as u32;
        let b = rgba[i * 4 + 2] as u32;
        let a = rgba[i * 4 + 3] as u32;
        *p = (a << 24) | (b << 16) | (g << 8) | r;
    }
}

/// 创建 32bpp top-down DIB，返回 (hdc, hbmp, bits 指针)
unsafe fn make_dib(hdc_screen: HDC, size: u32) -> Option<(HDC, HBITMAP, *mut c_void)> {
    let hdc = CreateCompatibleDC(hdc_screen);
    let mut bmi: BITMAPINFO = mem::zeroed();
    bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = size as i32;
    bmi.bmiHeader.biHeight = -(size as i32);
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    let mut bits: *mut c_void = std::ptr::null_mut();
    let hbmp =
        CreateDIBSection(hdc_screen, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
    if hbmp.is_null() || bits.is_null() {
        DeleteDC(hdc);
        return None;
    }
    Some((hdc, hbmp, bits))
}

/// 加载图片资源生成托盘图标（运行态白底黑墨；暂停态整体反色黑底白墨）
pub fn create(paused: bool) -> HICON {
    let size = ICON_SIZE;
    let rgba = if paused { PAUSED_RGBA } else { NORMAL_RGBA };
    unsafe {
        let hdc_screen = GetDC(std::ptr::null_mut());
        let (hdc, hbmp, bits) = match make_dib(hdc_screen, size) {
            Some(t) => t,
            None => {
                let _ = ReleaseDC(std::ptr::null_mut(), hdc_screen);
                return std::ptr::null_mut();
            }
        };

        write_icon_bits(bits, size, rgba);

        // ── 转 HICON：color=DIB，mask=全 0 单色位图（alpha 由 32bpp 决定） ──
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, std::ptr::null());
        let mut ii: ICONINFO = mem::zeroed();
        ii.fIcon = 1;
        ii.hbmMask = mask;
        ii.hbmColor = hbmp;
        let icon = CreateIconIndirect(&ii);

        DeleteObject(mask as _);
        DeleteObject(hbmp as _);
        DeleteDC(hdc);
        let _ = ReleaseDC(std::ptr::null_mut(), hdc_screen);

        icon
    }
}

/// 调试用：把图标资源 dump 成 BMP 文件，便于和任务栏图标对比。
pub fn dump_to_bmp(paused: bool, path: &std::path::Path) -> std::io::Result<()> {
    let rgba = if paused { PAUSED_RGBA } else { NORMAL_RGBA };
    let size = ICON_SIZE;
    let pixels: Vec<u32> = rgba
        .chunks_exact(4)
        .map(|c| {
            (c[3] as u32) << 24 | (c[2] as u32) << 16 | (c[1] as u32) << 8 | c[0] as u32
        })
        .collect();
    write_bmp(path, size, size, &pixels)
}

/// 写 32bpp BMP（BMP 默认 bottom-up，写文件时翻转行序）
fn write_bmp(path: &std::path::Path, w: u32, h: u32, pixels: &[u32]) -> std::io::Result<()> {
    use std::io::Write;
    let row_bytes = w * 4;
    let pad = (4 - (row_bytes % 4)) % 4;
    let img_size = (row_bytes + pad) * h;
    let file_size = 14 + 40 + img_size;

    let mut f = std::fs::File::create(path)?;
    f.write_all(b"BM")?;
    f.write_all(&file_size.to_le_bytes())?;
    f.write_all(&0u16.to_le_bytes())?;
    f.write_all(&0u16.to_le_bytes())?;
    f.write_all(&54u32.to_le_bytes())?;
    f.write_all(&40u32.to_le_bytes())?;
    f.write_all(&(w as i32).to_le_bytes())?;
    f.write_all(&(h as i32).to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?;
    f.write_all(&img_size.to_le_bytes())?;
    f.write_all(&2835u32.to_le_bytes())?;
    f.write_all(&2835u32.to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?;
    for row in (0..h).rev() {
        for x in 0..w {
            f.write_all(&pixels[(row * w + x) as usize].to_le_bytes())?;
        }
        for _ in 0..pad {
            f.write_all(&[0u8])?;
        }
    }
    Ok(())
}
