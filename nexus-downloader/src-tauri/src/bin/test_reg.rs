use winreg::{enums::HKEY_CURRENT_USER, RegKey};
use std::io::ErrorKind;

const AUTOSTART_REG_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_VALUE_NAME: &str = "Nexus Downloader";

fn main() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let command = "\"D:\\test\\Nexus Downloader.exe\" --tray".to_string();

    println!("Attempting to open subkey with KEY_SET_VALUE...");
    match hkcu.open_subkey_with_flags(AUTOSTART_REG_SUBKEY, winreg::enums::KEY_SET_VALUE) {
        Ok(key) => {
            println!("Opened subkey. Setting value...");
            match key.set_value(AUTOSTART_VALUE_NAME, &command) {
                Ok(_) => println!("Successfully wrote registry value."),
                Err(e) => println!("Error writing registry value: {}", e),
            }
        }
        Err(e) => println!("Error opening subkey: {}", e),
    }

    println!("\nAttempting to read it back...");
    match hkcu.open_subkey(AUTOSTART_REG_SUBKEY) {
        Ok(key) => {
            match key.get_value::<String, _>(AUTOSTART_VALUE_NAME) {
                Ok(val) => println!("Read value successfully: {}", val),
                Err(e) => println!("Error reading value: {} (Kind: {:?})", e, e.kind()),
            }
        }
        Err(e) => println!("Error opening for read: {}", e),
    }
}
