//! Extract Windows file-type icons via Shell API and return as base64 PNG data URLs.
//! Uses the same SHGetFileInfoW API that Windows Explorer uses internally.

use base64::Engine;

/// Tauri command: given a file extension (e.g. "zip", "exe", "pdf"),
/// return a `data:image/png;base64,...` string matching the current
/// Windows shell icon for that file type.
#[tauri::command]
pub fn get_type_icon(ext: String) -> Result<String, String> {
    #[cfg(windows)]
    {
        extract_icon_impl(&ext)
    }

    #[cfg(not(windows))]
    {
        let _ = ext;
        Err("get_type_icon is only available on Windows".into())
    }
}

// ---------------------------------------------------------------------------
// Windows implementation – Shell API icon extraction
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn extract_icon_impl(ext: &str) -> Result<String, String> {
    use std::mem;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows::Win32::UI::Shell::{self as shell, SHFILEINFOW, SHGFI_ICON, SHGFI_USEFILEATTRIBUTES};
    use windows::Win32::UI::WindowsAndMessaging as wam;
    use windows::core::PCWSTR;

    // Build a dummy filename like "file.zip" so the Shell knows the extension.
    let sanitized = ext.trim_start_matches('.');
    let wide: Vec<u16> = format!("file.{sanitized}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // 1. Ask the Shell for the HICON associated with this file type.
    let mut info = SHFILEINFOW::default();
    let ret = unsafe {
        shell::SHGetFileInfoW(
            PCWSTR::from_raw(wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_USEFILEATTRIBUTES,
        )
    };

    if ret == 0 {
        return Err(format!("SHGetFileInfoW returned 0 for extension '{ext}'"));
    }

    let hicon = info.hIcon;
    if hicon.0.is_null() {
        return Err(format!("Null HICON for extension '{ext}'"));
    }

    // 2. Convert HICON → RGBA pixel buffer → PNG → base64.
    let result = hicon_to_png_data_url(hicon);

    // Always release the icon handle.
    unsafe { let _ = wam::DestroyIcon(hicon); }

    result
}

/// Core conversion: HICON → RGBA → PNG → base64 data URL.
///
/// Uses CreateDIBSection + DrawIconEx instead of the older GetDIBits path
/// because GetDIBits on an icon's internal bitmap can lose the alpha channel,
/// producing fully transparent (white-looking) PNGs.
#[cfg(windows)]
fn hicon_to_png_data_url(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Result<String, String> {
    use std::mem;
    use std::ptr;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        self as gdi, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging as wam;

    // 1. Get the colour-bitmap handle from the icon so we can read its size.
    let mut icon_info = wam::ICONINFO::default();
    unsafe {
        wam::GetIconInfo(hicon, &mut icon_info)
            .map_err(|e| format!("GetIconInfo failed: {e}"))?;
    }

    let hbm_color = icon_info.hbmColor;
    let hbm_mask = icon_info.hbmMask;

    // 2. Read bitmap dimensions.
    let mut bmp = BITMAP::default();
    unsafe {
        gdi::GetObjectW(
            hbm_color.into(),
            mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut _ as *mut _),
        );
    }

    let width = bmp.bmWidth as u32;
    let height = bmp.bmHeight as u32;

    if width == 0 || height == 0 {
        unsafe {
            let _ = gdi::DeleteObject(hbm_color.into());
            let _ = gdi::DeleteObject(hbm_mask.into());
        }
        return Err("Icon bitmap has zero dimensions".into());
    }

    // 3. Create a fresh 32-bit top-down DIB section.
    //    DrawIconEx will render the HICON into this DIB with proper alpha.
    let bmp_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [Default::default()],
    };

    let hdc_screen = unsafe { gdi::GetDC(Some(HWND::default())) };
    let hdc = unsafe { gdi::CreateCompatibleDC(Some(hdc_screen)) };

    let mut pixel_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let hdib = unsafe {
        gdi::CreateDIBSection(
            Some(hdc),
            &bmp_info as *const BITMAPINFO,
            DIB_RGB_COLORS,
            &mut pixel_ptr,
            None,
            0,
        )
    }.map_err(|e| format!("CreateDIBSection failed: {e}"))?;
    if hdib.is_invalid() {
        unsafe {
            let _ = gdi::DeleteDC(hdc);
            let _ = gdi::ReleaseDC(Some(HWND::default()), hdc_screen);
            let _ = gdi::DeleteObject(hbm_color.into());
            let _ = gdi::DeleteObject(hbm_mask.into());
        }
        return Err("CreateDIBSection returned null DIB".into());
    }

    // Select the DIB into the memory DC so DrawIconEx can write to it.
    unsafe { gdi::SelectObject(hdc, hdib.into()) };

    // 4. Draw the icon onto the DIB with correct alpha handling.
    unsafe {
        let _ = wam::DrawIconEx(hdc, 0, 0, hicon, width as i32, height as i32, 0, None, wam::DI_NORMAL);
    }

    // 5. Read pixels from the DIB section's own buffer (accessible via pixel_ptr).
    let stride = width * 4;
    let buf_size = (stride * height) as usize;
    let mut buf: Vec<u8> = vec![0u8; buf_size];
    unsafe {
        ptr::copy_nonoverlapping(pixel_ptr as *const u8, buf.as_mut_ptr(), buf_size);
    }

    // 6. Cleanup GDI objects.
    unsafe {
        let _ = gdi::DeleteDC(hdc);
        let _ = gdi::ReleaseDC(Some(HWND::default()), hdc_screen);
        let _ = gdi::DeleteObject(hdib.into());
        let _ = gdi::DeleteObject(hbm_color.into());
        let _ = gdi::DeleteObject(hbm_mask.into());
    }

    // 7. Convert BGRA → RGBA (image::RgbaImage expects RGBA).
    for chunk in buf.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    // 8. Encode as PNG.
    let img = image::RgbaImage::from_raw(width, height, buf)
        .ok_or_else(|| "Failed to build RgbaImage from pixel buffer".to_string())?;

    let mut png_bytes: Vec<u8> = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("PNG encoding failed: {e}"))?;

    // 9. Base64-encode and return as a data URL.
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Ok(format!("data:image/png;base64,{b64}"))
}
