//! Registers Comic Reader as a candidate default handler for `.cbz`/`.cb7`/
//! `.cbr` (not the generic `.zip`/`.7z`/`.rar` extensions, which stay pointed
//! at whatever the user already has for archives in general).
//!
//! Windows has blocked silent `UserChoice` overwrites since Windows 8, so
//! this follows Microsoft's documented "Default Programs" pattern instead:
//! register a capability under `HKCU\Software\RegisteredApplications`, then
//! let the user finish the pick in Settings → Default apps (the caller opens
//! that panel right after this returns).
use std::io;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const PROG_ID: &str = "RustyComicReader.Comic";
const EXTENSIONS: &[&str] = &[".cbz", ".cb7", ".cbr"];

pub fn register_as_default() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let exe = exe.to_string_lossy();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // ProgID: what actually opens a file associated with it.
    let (prog_id_key, _) = hkcu.create_subkey(format!("Software\\Classes\\{PROG_ID}"))?;
    prog_id_key.set_value("", &"Comic Archive")?;
    let (icon_key, _) = hkcu.create_subkey(format!("Software\\Classes\\{PROG_ID}\\DefaultIcon"))?;
    icon_key.set_value("", &format!("{exe},0"))?;
    let (command_key, _) =
        hkcu.create_subkey(format!("Software\\Classes\\{PROG_ID}\\shell\\open\\command"))?;
    command_key.set_value("", &format!("\"{exe}\" \"%1\""))?;

    // Capability block + registration, so Comic Reader shows up as a choice
    // in Settings → Default apps for each extension below.
    let (caps_key, _) = hkcu.create_subkey("Software\\RustyComicReader\\Capabilities")?;
    caps_key.set_value("ApplicationName", &"Comic Reader")?;
    caps_key.set_value("ApplicationDescription", &"Dual-page comic book reader")?;
    let (assoc_key, _) =
        hkcu.create_subkey("Software\\RustyComicReader\\Capabilities\\FileAssociations")?;
    for ext in EXTENSIONS {
        assoc_key.set_value(*ext, &PROG_ID)?;
        // Classic per-extension association — takes effect immediately for
        // any extension that doesn't already have a `UserChoice` set.
        let (ext_key, _) = hkcu.create_subkey(format!("Software\\Classes\\{ext}"))?;
        ext_key.set_value("", &PROG_ID)?;
    }

    let (registered_apps, _) = hkcu.create_subkey("Software\\RegisteredApplications")?;
    registered_apps.set_value("Comic Reader", &"Software\\RustyComicReader\\Capabilities")?;

    Ok(())
}
