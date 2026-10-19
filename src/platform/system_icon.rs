#[cfg(windows)]
pub fn file_type_icon_png(ext: &str) -> Result<Vec<u8>, String> {
    file_icon_png(None, ext)
}

/// Shared by the local HTTP type-icon route and the trusted desktop IPC command.
#[cfg(windows)]
pub fn file_icon_png(path: Option<&str>, ext: &str) -> Result<Vec<u8>, String> {
    let _com = ComApartment::new();
    let ext = ext.trim().trim_start_matches('.').to_lowercase();
    if ext.len() > 32
        || !ext
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err("invalid file extension".into());
    }
    let existing = path.filter(|p| std::path::Path::new(p).is_file());
    // The real path also respects per-file icon handlers (EXE/ICO/LNK and others).
    if let Some(path) = existing {
        if let Ok(icon) = extract_shell_icon(path, false) {
            return consume_icon(icon);
        }
    }
    if let Ok(location) = default_icon_location(&ext) {
        // Store applications register an indirect PNG resource rather than an EXE/DLL.
        if location.path.starts_with('@') {
            if let Ok(bytes) = indirect_icon_png(&location.path) {
                return Ok(bytes);
            }
        } else if !location.path.contains("%1") {
            if let Ok(icon) = extract_default_icon(&location) {
                return consume_icon(icon);
            }
        }
    }
    let name = if ext.is_empty() {
        "file".into()
    } else {
        format!("file.{ext}")
    };
    consume_icon(extract_shell_icon(&name, true)?)
}

#[cfg(windows)]
fn consume_icon(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Result<Vec<u8>, String> {
    let result = hicon_to_png(hicon);
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(hicon);
    }
    result
}

#[cfg(windows)]
struct ComApartment(bool);

#[cfg(windows)]
impl ComApartment {
    fn new() -> Self {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

#[cfg(windows)]
fn indirect_icon_png(resource: &str) -> Result<Vec<u8>, String> {
    use windows::{Win32::UI::Shell::SHLoadIndirectString, core::PCWSTR};
    let wide: Vec<u16> = resource.encode_utf16().chain(Some(0)).collect();
    let mut output = vec![0u16; 32768];
    unsafe { SHLoadIndirectString(PCWSTR(wide.as_ptr()), &mut output, None) }
        .map_err(|e| format!("indirect icon resource: {e}"))?;
    let end = output.iter().position(|c| *c == 0).unwrap_or(output.len());
    let path = String::from_utf16_lossy(&output[..end]);
    let bytes = std::fs::read(path).map_err(|e| format!("icon asset: {e}"))?;
    let image = image::load_from_memory(&bytes).map_err(|e| format!("icon image: {e}"))?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png)
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
fn extract_shell_icon(
    path: &str,
    use_attributes: bool,
) -> Result<windows::Win32::UI::WindowsAndMessaging::HICON, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell;

    let wide_path: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut info = Shell::SHFILEINFOW::default();
    let mut flags = Shell::SHGFI_ICON;
    if use_attributes {
        flags |= Shell::SHGFI_USEFILEATTRIBUTES;
    }
    let result = unsafe {
        Shell::SHGetFileInfoW(
            windows::core::PCWSTR(wide_path.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<Shell::SHFILEINFOW>() as u32,
            flags,
        )
    };
    if result == 0 || info.hIcon.0.is_null() {
        return Err("Windows shell icon unavailable".into());
    }
    Ok(info.hIcon)
}

#[cfg(not(windows))]
pub fn file_type_icon_png(ext: &str) -> Result<Vec<u8>, String> {
    file_icon_png(None, ext)
}

#[cfg(not(windows))]
pub fn file_icon_png(path: Option<&str>, ext: &str) -> Result<Vec<u8>, String> {
    let _ = (path, ext);
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn store_application_icons_resolve_to_the_registered_asset() {
        let _com = ComApartment::new();
        for ext in ["zip", "jpg", "png"] {
            if let Ok(location) = default_icon_location(ext) {
                if location.path.starts_with('@') {
                    let expected = indirect_icon_png(&location.path)
                        .expect("registered Store icon must resolve");
                    assert_eq!(
                        file_type_icon_png(ext).unwrap(),
                        expected,
                        "{ext} must not silently become a blank document icon"
                    );
                }
            }
        }
    }

    #[test]
    fn shell_icons_cover_unknown_extensionless_and_actual_executables() {
        for ext in ["", "nexus_unknown", "7z", "rar", "pdf"] {
            let bytes = file_type_icon_png(ext).unwrap();
            let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
            assert!(image.pixels().any(|pixel| pixel.0[3] > 0), "{ext}");
        }
        for ext in ["../zip", "zip\\file", "zip\0", &"x".repeat(33)] {
            assert!(file_type_icon_png(ext).is_err());
        }
        if let Ok(path) = std::env::var("NEXUS_ICON_TEST_FILE") {
            let actual = file_icon_png(Some(&path), "exe").unwrap();
            assert_ne!(
                actual,
                file_type_icon_png("exe").unwrap(),
                "an application must use its own icon"
            );
        }
    }

    #[test]
    fn export_icons_when_explicitly_requested() {
        let Ok(dir) = std::env::var("NEXUS_ICON_TEST_OUTPUT") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for ext in ["zip", "7z", "rar", "jpg", "pdf"] {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("{ext}.png")),
                file_type_icon_png(ext).unwrap(),
            )
            .unwrap();
        }
    }
}
