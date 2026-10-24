//! Known-noise filtering. Windows writes constantly in the background;
//! a raw diff would drown the real changes. These patterns mark changes
//! that are operating-system bookkeeping, not the packaged app. Noise is
//! never deleted from a diff — it is flagged, and the classification
//! review shows it collapsed under "system noise" so the user can audit
//! every decision. (A false "noise" call must always be visible.)

/// Path fragments (matched case-insensitively, `\`-separated) that are
/// system bookkeeping wherever they appear.
const NOISE_PATH_FRAGMENTS: &[&str] = &[
    r"\appdata\local\microsoft\windows\prefetch",
    r"\windows\prefetch",
    r"\appdata\local\microsoft\windows\usrclass.dat",
    r"\appdata\local\microsoft\windows\explorer\thumbcache",
    r"\appdata\local\microsoft\windows\explorer\iconcache",
    r"\appdata\local\temp",
    r"\windows\temp",
    r"\appdata\local\microsoft\windows\inetcache",
    r"\appdata\local\microsoft\windows\webcache",
    r"\softwaredistribution\deliveryoptimization",
    r"\microsoft\windows\deliveryoptimization",
    r"\programdata\microsoft\search\data",
    r"\appdata\local\crashdumps",
    r"\programdata\microsoft\windows\wer",
    r"\windows\system32\winevt\logs",
    r"\windows\system32\config\",
    r"\ntuser.dat",
    r"\microsoft\windows\recent",
];

/// Registry key fragments that are shell/OS bookkeeping.
const NOISE_REG_FRAGMENTS: &[&str] = &[
    r"\software\microsoft\windows\currentversion\explorer\userassist",
    r"\software\microsoft\windows\currentversion\explorer\recentdocs",
    r"\software\microsoft\windows\currentversion\explorer\runmru",
    r"\software\classes\local settings\muicache",
    r"\software\microsoft\windows\shell\muicache",
    r"\software\microsoft\windows\currentversion\explorer\comdlg32",
    r"\software\microsoft\windows nt\currentversion\appcompatflags\compatibility assistant",
];

pub fn is_noise_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase().replace('/', "\\");
    NOISE_PATH_FRAGMENTS.iter().any(|f| p.contains(f))
        || p.ends_with(".tmp")
        || p.ends_with("ntuser.dat.log1")
        || p.ends_with("ntuser.dat.log2")
}

pub fn is_noise_reg(key: &str, _name: &str) -> bool {
    let k = key.to_ascii_lowercase().replace('/', "\\");
    NOISE_REG_FRAGMENTS.iter().any(|f| k.contains(f))
}

/// The uninstall-registration hive: an installer's presence there is
/// metadata (name/version/publisher for our forms), not app state.
pub fn is_uninstall_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains(r"\microsoft\windows\currentversion\uninstall\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefetch_and_muicache_are_noise_real_appdata_is_not() {
        assert!(is_noise_path(r"C:\Windows\Prefetch\WIDGET.EXE-1234.pf"));
        assert!(is_noise_path(
            r"C:\Users\X\AppData\Local\Microsoft\Windows\Explorer\thumbcache_32.db"
        ));
        assert!(!is_noise_path(
            r"C:\Users\X\AppData\Roaming\Acme\Widget\settings.ini"
        ));
        assert!(is_noise_reg(
            r"HKEY_CURRENT_USER\Software\Microsoft\Windows\Shell\MuiCache",
            "C:\\App\\a.exe.FriendlyAppName"
        ));
        assert!(!is_noise_reg(
            r"HKEY_CURRENT_USER\Software\Acme\Widget",
            "Mode"
        ));
    }

    #[test]
    fn delivery_optimization_and_search_index_are_noise() {
        assert!(is_noise_path(
            r"C:\Windows\SoftwareDistribution\DeliveryOptimization\Cache\a.bin"
        ));
        assert!(is_noise_path(
            r"C:\Users\X\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache\a.bin"
        ));
        assert!(is_noise_path(
            r"C:\ProgramData\Microsoft\Search\Data\Applications\Windows\Windows.edb"
        ));
        // A real program folder that merely mentions search is not noise.
        assert!(!is_noise_path(
            r"C:\Program Files\Acme\SearchWidget\search.exe"
        ));
    }

    #[test]
    fn uninstall_key_detection() {
        assert!(is_uninstall_key(
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Widget_is1"
        ));
        assert!(!is_uninstall_key(r"HKEY_CURRENT_USER\Software\Acme"));
    }
}
