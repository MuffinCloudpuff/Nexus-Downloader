#[cfg(windows)]
pub fn file_type_icon_png(ext: &str) -> Result<Vec<u8>, String> {
    use windows::Win32::UI::WindowsAndMessaging as wam;

    let sanitized = ext.trim().trim_start_matches('.');
    if sanitized.is_empty() {
        return Err("empty file extension".into());
    }

    let hicon = match default_icon_location(sanitized)
        .and_then(|location| extract_default_icon(&location))
    {
        Ok(icon) => icon,
        Err(_) => extract_associated_icon(sanitized)?,
    };

    if hicon.0.is_null() {
        return Err(format!("null HICON for extension '{ext}'"));
    }

    let result = hicon_to_png(hicon);
    unsafe {
        let _ = wam::DestroyIcon(hicon);
    }
    result
}

#[cfg(windows)]
#[derive(Debug)]
struct IconLocation {
    path: String,
    index: i32,
}

#[cfg(windows)]
fn default_icon_location(ext: &str) -> Result<IconLocation, String> {
    let prog_id = user_choice_prog_id(ext).or_else(|| hkcr_default_value(&format!(".{ext}")));
    let prog_id = prog_id.ok_or_else(|| format!("no ProgID found for extension '.{ext}'"))?;
    let raw = hkcr_default_value(&format!("{prog_id}\\DefaultIcon"))
        .ok_or_else(|| format!("no DefaultIcon registered for ProgID '{prog_id}'"))?;
    parse_icon_location(&raw)
}

#[cfg(windows)]
fn user_choice_prog_id(ext: &str) -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey(format!(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.{ext}\UserChoice"
        ))
        .ok()?;
    key.get_value::<String, _>("ProgId").ok()
}

#[cfg(windows)]
fn hkcr_default_value(path: &str) -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CLASSES_ROOT;

    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);
    let key = hkcr.open_subkey(path).ok()?;
    key.get_value::<String, _>("").ok()
}

#[cfg(windows)]
fn parse_icon_location(raw: &str) -> Result<IconLocation, String> {
    let expanded = expand_environment_strings(raw.trim());
    let without_quotes = expanded.trim().trim_matches('"');
    let (path, index) = if let Some((path, index)) = without_quotes.rsplit_once(',') {
        let parsed_index = index.trim().parse::<i32>().unwrap_or(0);
        (path.trim().trim_matches('"').to_string(), parsed_index)
    } else {
        (without_quotes.to_string(), 0)
    };

    if path.is_empty() {
        return Err(format!("empty DefaultIcon path from '{raw}'"));
    }
    Ok(IconLocation { path, index })
}

#[cfg(windows)]
fn expand_environment_strings(value: &str) -> String {
    let mut output = value.to_string();
    for (key, env_value) in std::env::vars() {
        output = output.replace(&format!("%{key}%"), &env_value);
    }
    output
}

#[cfg(windows)]
fn extract_default_icon(
    location: &IconLocation,
) -> Result<windows::Win32::UI::WindowsAndMessaging::HICON, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::SHDefExtractIconW;
    use windows::Win32::UI::WindowsAndMessaging::HICON;
    use windows::core::PCWSTR;

    let wide: Vec<u16> = std::ffi::OsStr::new(&location.path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut large = HICON::default();
    let mut small = HICON::default();
    let hr = unsafe {
        SHDefExtractIconW(
            PCWSTR::from_raw(wide.as_ptr()),
            location.index,
            0,
            Some(&mut large),
            Some(&mut small),
            0,
        )
    };
    if hr.is_err() {
        return Err(format!(
            "SHDefExtractIconW failed for '{}' index {}: {hr:?}",
            location.path, location.index
        ));
    }
    if !small.0.is_null() {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(small);
        }
    }
    if large.0.is_null() {
        return Err(format!(
            "SHDefExtractIconW returned null HICON for '{}' index {}",
            location.path, location.index
        ));
    }
    Ok(large)
}

#[cfg(windows)]
fn extract_associated_icon(
    ext: &str,
) -> Result<windows::Win32::UI::WindowsAndMessaging::HICON, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell;

    let file_path = temporary_probe_file(ext)?;
    let wide_path: Vec<u16> = file_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut icon_path = [0u16; 128];
    if wide_path.len() > icon_path.len() {
        let _ = std::fs::remove_file(&file_path);
        return Err(format!(
            "icon probe path is too long for ExtractAssociatedIconW: {}",
            file_path.display()
        ));
    }
    icon_path[..wide_path.len()].copy_from_slice(&wide_path);

    let mut icon_index = 0u16;
    let hicon = unsafe { Shell::ExtractAssociatedIconW(None, &mut icon_path, &mut icon_index) };
    let _ = std::fs::remove_file(&file_path);
    if hicon.0.is_null() {
        return Err(format!(
            "ExtractAssociatedIconW returned null HICON for '.{ext}'"
        ));
    }
    Ok(hicon)
}

#[cfg(windows)]
fn temporary_probe_file(ext: &str) -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join("gd3-system-icons");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create icon temp dir: {error}"))?;
    let file_path = dir.join(format!("probe.{ext}"));
    if !file_path.exists() {
        std::fs::write(&file_path, [])
            .map_err(|error| format!("failed to create icon probe file: {error}"))?;
    }
    Ok(file_path)
}

#[cfg(not(windows))]
pub fn file_type_icon_png(ext: &str) -> Result<Vec<u8>, String> {
    let _ = ext;
    Err("system file-type icons are only available on Windows".into())
}

#[cfg(windows)]
fn hicon_to_png(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        self as gdi, BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging as wam;

    let mut icon_info = wam::ICONINFO::default();
    unsafe {
        wam::GetIconInfo(hicon, &mut icon_info)
            .map_err(|error| format!("GetIconInfo failed: {error}"))?;
    }

    let hbm_color = icon_info.hbmColor;
    let hbm_mask = icon_info.hbmMask;

    let result = (|| {
        let mut bmp = BITMAP::default();
        unsafe {
            gdi::GetObjectW(
                hbm_color.into(),
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bmp as *mut _ as *mut _),
            );
        }

        let width = bmp.bmWidth as u32;
        let height = bmp.bmHeight as u32;
        if width == 0 || height == 0 {
            return Err("icon bitmap has zero dimensions".into());
        }

        let bmp_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
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
        }
        .map_err(|error| format!("CreateDIBSection failed: {error}"))?;

        if hdib.is_invalid() {
            unsafe {
                let _ = gdi::DeleteDC(hdc);
                let _ = gdi::ReleaseDC(Some(HWND::default()), hdc_screen);
            }
            return Err("CreateDIBSection returned null DIB".into());
        }

        unsafe {
            gdi::SelectObject(hdc, hdib.into());
            let _ = wam::DrawIconEx(
                hdc,
                0,
                0,
                hicon,
                width as i32,
                height as i32,
                0,
                None,
                wam::DI_NORMAL,
            );
        }

        let byte_len = (width * height * 4) as usize;
        let mut buf = vec![0u8; byte_len];
        unsafe {
            std::ptr::copy_nonoverlapping(pixel_ptr as *const u8, buf.as_mut_ptr(), byte_len);
        }

        unsafe {
            let _ = gdi::DeleteDC(hdc);
            let _ = gdi::ReleaseDC(Some(HWND::default()), hdc_screen);
            let _ = gdi::DeleteObject(hdib.into());
        }

        for pixel in buf.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        let img = image::RgbaImage::from_raw(width, height, buf)
            .ok_or_else(|| "failed to build RgbaImage from icon pixels".to_string())?;
        let mut png_bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|error| format!("PNG encoding failed: {error}"))?;
        Ok(png_bytes)
    })();

    unsafe {
        let _ = gdi::DeleteObject(hbm_color.into());
        let _ = gdi::DeleteObject(hbm_mask.into());
    }

    result
}
