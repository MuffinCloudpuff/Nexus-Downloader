use std::os::windows::process::CommandExt;

fn main() {
    let script = format!(
        "$WshShell = New-Object -comObject WScript.Shell; \
         $Shortcut = $WshShell.CreateShortcut('{}'); \
         $Shortcut.TargetPath = '{}'; \
         $Shortcut.Arguments = '--tray'; \
         $Shortcut.Save()",
        "C:\\Users\\Liu\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\Nexus Downloader Test.lnk",
        "D:\\test.exe"
    );

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    println!("Running script...");
    match std::process::Command::new("powershell")
        .args(&["-NoProfile", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                println!(
                    "Failed! Stderr: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            } else {
                println!("Success!");
            }
        }
        Err(e) => println!("Error: {}", e),
    }
}
