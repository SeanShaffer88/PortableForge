use std::ffi::{c_void, OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::thread;
use std::time::{Duration, Instant};

use crate::compile::windows::{file_digest, local_directory, require_regular_path};

const INSTALL: &str = r"C:\Program Files\Sandboxie-Plus";
const TOOLS: &[(&str, &str)] = &[
    (
        "SbieDll.dll",
        "12643b9b87a3813ef7865f7b9a7e2e53e5af8b6d763a08d2ab0fd068187a5ebd",
    ),
    (
        "SbieSvc.exe",
        "12b8ac8462f65d2c26504e01916e4d9411a9ab7e0a64e821bc7a6755c8dacb43",
    ),
    (
        "SbieDrv.sys",
        "96f220d2ff4b27bfaceeb7bc1f19621c5f1b9e594bfd8060d2d7ac6516cd6ee9",
    ),
    (
        "Start.exe",
        "55820d96077bd59747cae8862cc5f642a43d16a1e7a62f29e1c6d7b530c60060",
    ),
    (
        "Templates.ini",
        "c9e06a889ed23f1689b8dfca1210015c256ea71477b2e33b34feb0dc6fb07933",
    ),
];
const DRIVER_VERSION: &str = "5.73.8";
const DRIVER_ABI: u32 = 0x57230;
const STATUS_RESOURCE_NAME_NOT_FOUND: i32 = 0xc000008b_u32 as i32;
const CONF_NO_EXPAND: u32 = 0x20000000;
const DIRECT_CONF: u32 = 0x70000000; // NO_GLOBAL | NO_EXPAND | NO_TEMPLS
const BOX_CHARS: usize = 40; // v1.18.8 common/defines.h
const PATH_CHARS: usize = 4096;
const OWNER_KEY: &str = "PortableForgeOwner";
const CONFIG_LEVEL: (&str, &str) = ("ConfigLevel", "10");
const POLICY: &[(&str, &str)] = &[
    ("DropAdminRights", "y"),
    ("NoSecurityIsolation", "n"),
    ("DisableFileFilter", "n"),
    ("DisableKeyFilter", "n"),
    ("DisableObjectFilter", "n"),
    ("BlockNetworkFiles", "y"),
    ("AllowNetworkAccess", "n"),
    ("ClosedFilePath", r"\Device\Afd*"),
    ("AutoRecover", "n"),
    ("AutoDelete", "n"),
    ("NeverDelete", "n"),
    ("SeparateUserFolders", "y"),
];

type Handle = *mut c_void;
type Version = unsafe extern "system" fn(*mut u16, *mut u32) -> i32;
type DriverInfo = unsafe extern "system" fn(u32, *mut c_void, u32) -> i32;
type Home = unsafe extern "system" fn(*mut u16, u32, *mut u16, u32) -> i32;
type Conf = unsafe extern "system" fn(*const u16, *const u16, u32, *mut u16, u32) -> i32;
type Update = unsafe extern "system" fn(u16, *const u16, *const u16, *const u16, *const u16) -> u32;
type Enabled = unsafe extern "system" fn(*const u16) -> i32;
type BoxPath = unsafe extern "system" fn(
    *const u16,
    *mut u16,
    *mut u16,
    *mut u16,
    *mut u32,
    *mut u32,
    *mut u32,
) -> i32;
type ProcessPath = unsafe extern "system" fn(
    Handle,
    *mut u16,
    *mut u16,
    *mut u16,
    *mut u32,
    *mut u32,
    *mut u32,
) -> i32;
type Run = unsafe extern "system" fn(
    *const u16,
    *const u16,
    *const u16,
    u32,
    *mut StartupInfo,
    *mut ProcessInformation,
) -> i32;
type Query = unsafe extern "system" fn(Handle, *mut u16, *mut u16, *mut u16, *mut u32) -> i32;
type Info = unsafe extern "system" fn(Handle, u32) -> u64;
type Enumerate = unsafe extern "system" fn(*const u16, u8, u32, *mut u32, *mut u32) -> i32;
type Kill = unsafe extern "system" fn(u32, *const u16) -> u8;

#[repr(C)]
struct StartupInfo {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    position_and_size: [u32; 7],
    flags: u32,
    show_window: u16,
    reserved_bytes: u16,
    reserved_data: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(path: *const u16, file: Handle, flags: u32) -> Handle;
    fn FreeLibrary(module: Handle) -> i32;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const c_void;
    fn GetModuleFileNameW(module: Handle, path: *mut u16, size: u32) -> u32;
    fn QueryDosDeviceW(device: *const u16, target: *mut u16, size: u32) -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentProcess() -> Handle;
    fn GetProcessId(process: Handle) -> u32;
    fn GetProcessIdOfThread(thread: Handle) -> u32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        path: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn ResumeThread(thread: Handle) -> u32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, exit: *mut u32) -> i32;
    fn TerminateProcess(process: Handle, exit: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> Handle;
    fn OpenServiceW(manager: Handle, name: *const u16, access: u32) -> Handle;
    fn QueryServiceStatus(service: Handle, status: *mut u32) -> i32;
    fn CloseServiceHandle(handle: Handle) -> i32;
    fn RegOpenKeyExW(
        key: Handle,
        path: *const u16,
        options: u32,
        access: u32,
        result: *mut Handle,
    ) -> i32;
    fn RegCloseKey(key: Handle) -> i32;
}

struct Library(Handle);

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: this is the reference from our successful LoadLibraryExW.
        unsafe { FreeLibrary(self.0) };
    }
}

struct ProcessHandle(Handle);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this wrapper owns one Win32 process or thread handle.
            unsafe { CloseHandle(self.0) };
        }
    }
}

struct ServiceHandle(Handle);

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns a non-null handle from the SCM.
        unsafe { CloseServiceHandle(self.0) };
    }
}

pub(super) struct Sandboxie {
    version: Version,
    driver_info: DriverInfo,
    home: Home,
    conf: Conf,
    update: Update,
    enabled: Enabled,
    box_path: BoxPath,
    process_path: ProcessPath,
    launch: Run,
    query: Query,
    info: Info,
    enumerate: Enumerate,
    kill: Kill,
    _library: Library,
}

pub(super) struct VerifiedExit {
    pub(super) pid: u32,
    pub(super) exit_code: u32,
}

#[derive(PartialEq, Eq)]
struct Paths {
    file: String,
    key: String,
    ipc: String,
}

impl Sandboxie {
    pub(super) fn load() -> Result<Self, String> {
        service_running("SbieSvc")?;
        service_running("SbieDrv")?;
        let (driver, install) = Self::open()?;
        let mut version = [0; 16];
        let mut abi = 0;
        // SAFETY: both writable outputs have the vendor ABI's required sizes.
        checked("query the installed Sandboxie driver", unsafe {
            (driver.version)(version.as_mut_ptr(), &mut abi)
        })?;
        if text(&version)? != DRIVER_VERSION || abi != DRIVER_ABI {
            return Err(
                "The running Sandboxie driver is not from supported Sandboxie-Plus 1.18.8 x64."
                    .into(),
            );
        }
        let mut features = 0_u32;
        // SAFETY: feature query class 0 writes one ULONG in the pinned ABI.
        checked("check Sandboxie driver isolation support", unsafe {
            (driver.driver_info)(0, (&mut features as *mut u32).cast(), 4)
        })?;
        if features & 0x3000_0000 != 0x1000_0000 {
            return Err(
                "Sandboxie did not activate supported isolation for this Windows build.".into(),
            );
        }
        let mut home = [0; PATH_CHARS];
        // SAFETY: the DOS path output is a writable WCHAR buffer of the given length.
        checked("query the Sandboxie driver installation path", unsafe {
            (driver.home)(null_mut(), 0, home.as_mut_ptr(), PATH_CHARS as u32)
        })?;
        if local_directory(Path::new(&text(&home)?))? != install {
            return Err(
                "The running Sandboxie driver uses a different installation folder.".into(),
            );
        }
        Ok(driver)
    }

    fn open() -> Result<(Self, PathBuf), String> {
        if !cfg!(target_arch = "x86_64") {
            return Err(
                "This Sandboxie workflow requires x64 PortableForge and Sandboxie-Plus 1.18.8."
                    .into(),
            );
        }
        let install = local_directory(Path::new(INSTALL))?;
        let dll = install.join("SbieDll.dll");
        // Hold read-only sharing while checking the DLL and taking the loader reference.
        let _lock = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&dll)
            .map_err(|e| format!("Could not hold the installed Sandboxie library: {e}"))?;
        for (name, digest) in TOOLS {
            let path = install.join(name);
            require_regular_path(&path, &install)?;
            if file_digest(&path)?.0 != *digest {
                return Err(format!(
                    "The installed Sandboxie file {name} differs from supported Plus 1.18.8 x64."
                ));
            }
        }
        let dll_name = wide(dll.as_os_str())?;
        // SAFETY: absolute, verified DLL name, NUL terminated. Dependencies are
        // resolved from its verified installation directory and System32 only.
        let handle = unsafe { LoadLibraryExW(dll_name.as_ptr(), null_mut(), 0x100 | 0x800) };
        if handle.is_null() {
            return Err(os_error("load the verified Sandboxie library"));
        }
        let library = Library(handle);
        let mut actual = [0; PATH_CHARS];
        // SAFETY: live module handle and bounded writable output.
        let length = unsafe { GetModuleFileNameW(handle, actual.as_mut_ptr(), PATH_CHARS as u32) };
        if length == 0 || length as usize >= actual.len() {
            return Err("Could not read the loaded Sandboxie library path.".into());
        }
        let observed = PathBuf::from(OsString::from_wide(&actual[..length as usize]));
        let canonical = |path: &Path| {
            fs::canonicalize(path).map_err(|e| format!("Could not resolve {}: {e}", path.display()))
        };
        if canonical(&observed)? != canonical(&dll)? {
            return Err(
                "The loaded Sandboxie library does not match the verified installation path."
                    .into(),
            );
        }
        macro_rules! export {
            ($name:literal, $kind:ty) => {{
                // SAFETY: the pinned x64 DLL exports this NUL-terminated name
                // with exactly the signature declared in v1.18.8's headers.
                let address = unsafe { GetProcAddress(handle, concat!($name, "\0").as_ptr()) };
                if address.is_null() {
                    return Err(format!(
                        "Sandboxie is missing the required {} export.",
                        $name
                    ));
                }
                unsafe { std::mem::transmute::<*const c_void, $kind>(address) }
            }};
        }
        Ok((
            Self {
                version: export!("SbieApi_GetVersionEx", Version),
                driver_info: export!("SbieApi_QueryDrvInfo", DriverInfo),
                home: export!("SbieApi_GetHomePath", Home),
                conf: export!("SbieApi_QueryConf", Conf),
                update: export!("SbieDll_UpdateConf", Update),
                enabled: export!("SbieApi_IsBoxEnabled", Enabled),
                box_path: export!("SbieApi_QueryBoxPath", BoxPath),
                process_path: export!("SbieApi_QueryProcessPath", ProcessPath),
                launch: export!("SbieDll_RunSandboxed", Run),
                query: export!("SbieApi_QueryProcess", Query),
                info: export!("SbieApi_QueryProcessInfo", Info),
                enumerate: export!("SbieApi_EnumProcessEx", Enumerate),
                kill: export!("SbieDll_KillAll", Kill),
                _library: library,
            },
            install,
        ))
    }

    pub(super) fn provision(&self, name: &str, root: &Path) -> Result<(), String> {
        box_name(name)?;
        let root_text = validate_root(root)?;
        match root.symlink_metadata() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not verify the proposed box path: {error}")),
            Ok(_) => return Err("The proposed Sandboxie box storage already exists.".into()),
        }
        if self.setting(Some(name), None, DIRECT_CONF)?.is_some() || self.count(name)? != 0 {
            return Err("The proposed Sandboxie box already exists.".into());
        }
        self.verify_global()?;
        let mut config_level_written = false;
        let configured = (|| {
            // SandMan migrates boxes with an older or absent ConfigLevel,
            // which can add recovery folders and compatibility templates.
            self.set(name, CONFIG_LEVEL.0, Some(CONFIG_LEVEL.1))?;
            config_level_written = true;
            self.set(name, OWNER_KEY, Some(root_text))?;
            self.assert_owner(name, root)?;
            self.set(name, "FileRootPath", Some(root_text))?;
            for (key, value) in POLICY {
                self.set(name, key, Some(value))?;
            }
            self.set(name, "Enabled", Some("y"))?;
            self.verify_box(name, root)?;
            Ok(())
        })();
        if let Err(error) = configured {
            if self.setting(Some(name), None, DIRECT_CONF).ok() == Some(None) {
                return Err(error);
            }
            let cleanup = if config_level_written {
                self.rollback_provision(name, root)
            } else {
                self.remove_config(name, root)
            };
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!(
                    "{error}\nCould not roll back the owned box {name}: {cleanup}"
                )),
            };
        }
        Ok(())
    }

    pub(super) fn run(
        &self,
        name: &str,
        root: &Path,
        image: &Path,
        args: &[OsString],
        cwd: &Path,
        timeout: Duration,
    ) -> Result<VerifiedExit, String> {
        let paths = self.verify_box(name, root)?;
        let image_parent = image
            .parent()
            .ok_or("Choose an absolute executable path.")?;
        let parent = local_directory(image_parent)?;
        let image = parent.join(image.file_name().ok_or("Choose an executable file.")?);
        require_regular_path(&image, &parent)?;
        let _image_lock = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&image)
            .map_err(|e| format!("Could not hold the executable while launching it: {e}"))?;
        let dir = wide(local_directory(cwd)?.as_os_str())?;
        let command = command_line(image.as_os_str(), args)?;
        let name_wide = box_name(name)?;
        let milliseconds = timeout.as_millis().min((u32::MAX - 1) as u128) as u32;
        // SAFETY: Win32 STARTUPINFOW/PROCESS_INFORMATION are C layouts with a
        // valid all-zero initial state. The service accepts these flags.
        let mut startup: StartupInfo = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<StartupInfo>() as u32;
        let mut process: ProcessInformation = unsafe { std::mem::zeroed() };
        // SAFETY: all buffers remain live and NUL-terminated. CREATE_SUSPENDED
        // is honored by the pinned ProcessServer before returning the handles.
        if unsafe {
            (self.launch)(
                name_wide.as_ptr(),
                command.as_ptr(),
                dir.as_ptr(),
                4,
                &mut startup,
                &mut process,
            )
        } == 0
        {
            return Err(os_error("create the suspended Sandboxie process"));
        }
        let child = ProcessHandle(process.process);
        let primary = ProcessHandle(process.thread);
        if child.0.is_null() {
            return Err("Sandboxie did not return the suspended process handle.".into());
        }
        // SAFETY: both non-null handles were returned by the sandbox service.
        if primary.0.is_null()
            || process.pid == 0
            || unsafe { GetProcessId(child.0) } != process.pid
            || unsafe { GetProcessIdOfThread(primary.0) } != process.pid
        {
            return Err(reject_child(
                child.0,
                "Sandboxie returned inconsistent process identity. The process was not resumed."
                    .into(),
            ));
        }
        if let Err(error) = self.verify_process(name, &paths, process.pid, child.0, &image, false) {
            return Err(reject_child(child.0, error));
        }
        // SAFETY: driver membership, configured roots, isolation flags and the
        // exact retained process image have been checked while still suspended.
        let count = unsafe { ResumeThread(primary.0) };
        if count != 1 {
            return Err(reject_child(
                child.0,
                format!("Unexpected Sandboxie thread suspension count {count}."),
            ));
        }
        // SAFETY: the service returned a retained, waitable process handle.
        let wait = unsafe { WaitForSingleObject(child.0, milliseconds) };
        match wait {
            0 => {
                let mut exit = 0;
                // SAFETY: the process is signaled and the exit-code output is writable.
                if unsafe { GetExitCodeProcess(child.0, &mut exit) } == 0 {
                    return Err(os_error("read the boxed process exit code"));
                }
                Ok(VerifiedExit {
                    pid: process.pid,
                    exit_code: exit,
                })
            }
            258 => Err(
                "The boxed process exceeded its time limit. The owned box needs cleanup.".into(),
            ),
            _ => Err(os_error("wait for the boxed process")),
        }
    }

    pub(super) fn verify_current(name: &str, root: &Path, image: &Path) -> Result<(), String> {
        let (driver, _) = Self::open()?;
        // The host launch checked the full effective policy before resuming
        // this helper. Some configuration queries are restricted in the box.
        let paths = driver.roots(name, root)?;
        // SAFETY: these functions return the calling process's own identity.
        let (pid, current) = unsafe { (GetCurrentProcessId(), GetCurrentProcess()) };
        driver.verify_process(name, &paths, pid, current, image, true)
    }

    pub(super) fn quiescent(&self, name: &str, timeout: Duration) -> Result<(), String> {
        let until = Instant::now() + timeout;
        loop {
            if self.count(name)? == 0 {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(format!("Sandboxie box {name} still has running processes."));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub(super) fn stop(&self, name: &str, root: &Path) -> Result<(), String> {
        self.assert_owner(name, root)?;
        self.roots(name, root)?;
        if self.count(name)? != 0 {
            let box_wide = box_name(name)?;
            let session = current_session()?;
            // SAFETY: exactly this fresh, verified-owned box in this session.
            if unsafe { (self.kill)(session, box_wide.as_ptr()) } == 0 {
                return Err(format!("Sandboxie refused to stop the owned box {name}."));
            }
        }
        self.quiescent(name, Duration::from_secs(10))
    }

    pub(super) fn remove(&self, name: &str, root: &Path) -> Result<(), String> {
        self.assert_owner(name, root)?;
        let paths = self.roots(name, root)?;
        self.quiescent(name, Duration::ZERO)?;
        let key = paths
            .key
            .strip_prefix(r"\REGISTRY\USER\")
            .ok_or("Sandboxie reported an unexpected registry root.")?;
        let until = Instant::now() + Duration::from_secs(10);
        while registry_key_exists(key)? {
            if Instant::now() >= until {
                return Err(format!(
                    "Sandboxie box {name} still has a mounted registry hive."
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
        self.remove_config(name, root)
    }

    fn remove_config(&self, name: &str, root: &Path) -> Result<(), String> {
        self.assert_owner(name, root)?;
        self.delete_idle_config(name)
    }

    fn rollback_provision(&self, name: &str, root: &Path) -> Result<(), String> {
        // The first write prevents SandMan from migrating the fresh section.
        // If recording ownership then failed, accept only that exact partial
        // state, with no enabled box and no storage to remove.
        let settings = self.setting_names(Some(name))?;
        if matches!(settings.as_slice(), [key] if key == CONFIG_LEVEL.0)
            && self
                .setting(Some(name), Some(CONFIG_LEVEL.0), DIRECT_CONF)?
                .as_deref()
                == Some(CONFIG_LEVEL.1)
            && self
                .setting(Some(name), Some(CONFIG_LEVEL.0), DIRECT_CONF + 1)?
                .is_none()
            && self
                .setting(Some(name), Some("Enabled"), CONF_NO_EXPAND)?
                .is_none()
            && matches!(root.symlink_metadata(), Err(error) if error.kind() == io::ErrorKind::NotFound)
        {
            return self.delete_idle_config(name);
        }
        self.remove_config(name, root)
    }

    fn delete_idle_config(&self, name: &str) -> Result<(), String> {
        self.quiescent(name, Duration::ZERO)?;
        self.set(name, "*", None)?;
        if self.setting(Some(name), None, DIRECT_CONF)?.is_some() {
            return Err(format!(
                "The owned Sandboxie box {name} remains configured."
            ));
        }
        Ok(())
    }

    fn verify_box(&self, name: &str, root: &Path) -> Result<Paths, String> {
        self.assert_owner(name, root)?;
        self.verify_global()?;
        for (key, value) in POLICY.iter().copied().chain([
            CONFIG_LEVEL,
            ("Enabled", "y"),
            ("FileRootPath", validate_root(root)?),
        ]) {
            // QueryConf expands DOS paths into NT paths by default. Read the
            // effective literal here, then verify its resolved box root below.
            let effective_index = if key == "FileRootPath" {
                CONF_NO_EXPAND
            } else {
                0
            };
            if self.setting(Some(name), Some(key), DIRECT_CONF)?.as_deref() != Some(value)
                || self
                    .setting(Some(name), Some(key), DIRECT_CONF + 1)?
                    .is_some()
                || self
                    .setting(Some(name), Some(key), effective_index)?
                    .as_deref()
                    != Some(value)
            {
                return Err(format!(
                    "Sandboxie did not apply the required box setting {key}."
                ));
            }
        }
        for key in self.setting_names(Some(name))? {
            if !POLICY
                .iter()
                .any(|(known, _)| key.eq_ignore_ascii_case(known))
                && ![OWNER_KEY, CONFIG_LEVEL.0, "Enabled", "FileRootPath"]
                    .iter()
                    .any(|known| key.eq_ignore_ascii_case(known))
            {
                return Err(format!(
                    "The dedicated Sandboxie box has an unexpected setting {key}."
                ));
            }
        }
        let box_wide = box_name(name)?;
        // SAFETY: fixed-size, NUL-terminated box-name buffer.
        checked("confirm the dedicated Sandboxie box is enabled", unsafe {
            (self.enabled)(box_wide.as_ptr())
        })?;
        self.roots(name, root)
    }

    fn verify_global(&self) -> Result<(), String> {
        // Driver-enforced defaults from the pinned vendor installation are
        // supported. Explicit local/global compatibility exceptions are not.
        for key in self.setting_names(Some("GlobalSettings"))? {
            let lower = key.to_ascii_lowercase();
            if !matches!(
                lower.as_str(),
                "editadminonly"
                    | "confpassword"
                    | "networkenablewfp"
                    | "filemaxtotal"
                    | "forceprocesstimer"
                    | "autocreate"
                    | "autostartsandman"
                    | "maxprocesses"
                    | "maxsize"
            ) && !lower.starts_with("sbiectrl_")
            {
                return Err(format!(
                    "Sandboxie has an unqualified global setting {key}."
                ));
            }
        }
        // Explicit local sections come before the vendor template sections in
        // the driver's unexpanded enumeration. Reject local definitions that
        // can replace or extend settings from the pinned Templates.ini file.
        for section in self.setting_names(None)? {
            let lower = section.to_ascii_lowercase();
            if lower.starts_with("template_")
                || matches!(lower.as_str(), "defaulttemplates" | "templatesettings")
            {
                return Err(format!(
                    "Sandboxie has an unqualified local template section {section}."
                ));
            }
        }
        Ok(())
    }

    fn assert_owner(&self, name: &str, root: &Path) -> Result<(), String> {
        box_name(name)?;
        if self
            .setting(Some(name), Some(OWNER_KEY), DIRECT_CONF)?
            .as_deref()
            != Some(validate_root(root)?)
        {
            return Err(format!(
                "Sandboxie box {name} no longer matches this run's ownership record."
            ));
        }
        Ok(())
    }

    fn setting_names(&self, section: Option<&str>) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        for index in 0..128 {
            match self.setting(section, None, DIRECT_CONF | index)? {
                Some(name) => names.push(name),
                None => return Ok(names),
            }
        }
        Err("Sandboxie configuration has too many explicit settings to verify.".into())
    }

    fn setting(
        &self,
        section: Option<&str>,
        key: Option<&str>,
        index: u32,
    ) -> Result<Option<String>, String> {
        let section = section.map(fixed::<66>).transpose()?;
        let key = key.map(fixed::<66>).transpose()?;
        let mut output = [0; PATH_CHARS];
        // SAFETY: source ABI probes 64 WCHAR for section and setting pointers.
        // These are null or padded to 66, and output size is expressed in bytes.
        let status = unsafe {
            (self.conf)(
                section.as_ref().map_or(null(), |w| w.as_ptr()),
                key.as_ref().map_or(null(), |w| w.as_ptr()),
                index,
                output.as_mut_ptr(),
                (output.len() * 2) as u32,
            )
        };
        if status == STATUS_RESOURCE_NAME_NOT_FOUND {
            Ok(None)
        } else {
            checked("read Sandboxie configuration", status)?;
            text(&output).map(Some)
        }
    }

    fn set(&self, name: &str, key: &str, value: Option<&str>) -> Result<(), String> {
        let section = fixed::<66>(name)?;
        let key_wide = fixed::<66>(key)?;
        let value = value.map(|value| wide(OsStr::new(value))).transpose()?;
        // SAFETY: this writes only to the validated dedicated box. The strings
        // fit the vendor request's section/setting fields and are terminated.
        let status = unsafe {
            (self.update)(
                b's' as u16,
                null(),
                section.as_ptr(),
                key_wide.as_ptr(),
                value.as_ref().map_or(null(), |w| w.as_ptr()),
            )
        };
        checked(&format!("update Sandboxie setting {key}"), status as i32)
    }

    fn count(&self, name: &str) -> Result<u32, String> {
        let name = box_name(name)?;
        let mut count = 0;
        // SAFETY: fixed box buffer, query all sessions, count-only avoids truncation.
        checked(
            "enumerate all processes in the dedicated Sandboxie box",
            unsafe { (self.enumerate)(name.as_ptr(), 1, u32::MAX, null_mut(), &mut count) },
        )?;
        Ok(count)
    }

    fn roots(&self, name: &str, root: &Path) -> Result<Paths, String> {
        let name_wide = box_name(name)?;
        let paths = read_roots(|file, key, ipc, file_len, key_len, ipc_len| {
            // SAFETY: box name and output buffers have the vendor ABI's sizes.
            unsafe {
                (self.box_path)(
                    name_wide.as_ptr(),
                    file,
                    key,
                    ipc,
                    file_len,
                    key_len,
                    ipc_len,
                )
            }
        })?;
        let registry_suffix = format!("_{name}");
        let key_user = paths
            .key
            .strip_prefix(r"\REGISTRY\USER\Sandbox_")
            .and_then(|suffix| suffix.strip_suffix(&registry_suffix));
        let ipc_suffix = format!(r"\{name}\Session_{}", current_session()?);
        let ipc_user = paths
            .ipc
            .strip_prefix(r"\Sandbox\")
            .and_then(|suffix| suffix.strip_suffix(&ipc_suffix));
        if !same_file_root(&paths.file, validate_root(root)?)?
            || key_user.map_or(true, |user| user.is_empty() || user.contains('\\'))
            || key_user != ipc_user
        {
            return Err(
                "Sandboxie reported storage or namespace roots outside the owned box.".into(),
            );
        }
        Ok(paths)
    }

    fn verify_process(
        &self,
        name: &str,
        paths: &Paths,
        pid: u32,
        process: Handle,
        image: &Path,
        initialized: bool,
    ) -> Result<(), String> {
        let pid_handle = pid as usize as Handle; // vendor API takes PID encoded as HANDLE
        let mut box_wide = [0; BOX_CHARS];
        let mut session = u32::MAX;
        // SAFETY: pinned ABI expects BOXNAME_COUNT writable WCHARs and one ULONG.
        checked(
            "verify process membership with the Sandboxie driver",
            unsafe {
                (self.query)(
                    pid_handle,
                    box_wide.as_mut_ptr(),
                    null_mut(),
                    null_mut(),
                    &mut session,
                )
            },
        )?;
        // SAFETY: the PID is validated by the driver, with the flags selector 0.
        let flags = unsafe { (self.info)(pid_handle, 0) };
        // The driver applies DropAdminRights during Process_NotifyImage, after
        // registering the suspended process. Before resume we verified the
        // effective policy. The initialized helper must also report the flag.
        let required_flags = if initialized { 0x81 } else { 1 };
        const INSECURE_FLAGS: u64 = 0x0100_0000 | 0x8000_0000 | 0x2000 | 0x0200;
        if text(&box_wide)? != name
            || session != current_session()?
            || flags & required_flags != required_flags
            || flags & INSECURE_FLAGS != 0
        {
            return Err("The process did not enter the exact dedicated Sandboxie box with standard isolation.".into());
        }
        let actual = read_roots(|file, key, ipc, file_len, key_len, ipc_len| {
            // SAFETY: validated PID and writable vendor ABI output buffers.
            unsafe { (self.process_path)(pid_handle, file, key, ipc, file_len, key_len, ipc_len) }
        })?;
        if actual != *paths {
            return Err("The boxed process uses different storage or namespaces.".into());
        }
        let mut path = [0; PATH_CHARS];
        let mut len = PATH_CHARS as u32;
        // SAFETY: process is a retained service-returned handle or current pseudo-handle.
        if unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut len) } == 0
            || len == 0
            || len as usize >= path.len()
        {
            return Err(os_error("check the boxed process image"));
        }
        let observed = PathBuf::from(OsString::from_wide(&path[..len as usize]));
        let expected = fs::canonicalize(image)
            .map_err(|e| format!("Could not resolve expected process image: {e}"))?;
        if fs::canonicalize(observed)
            .map_err(|e| format!("Could not resolve launched process image: {e}"))?
            != expected
        {
            return Err("The launched process image differs from the selected executable.".into());
        }
        Ok(())
    }
}

fn read_roots(
    query: impl FnOnce(*mut u16, *mut u16, *mut u16, *mut u32, *mut u32, *mut u32) -> i32,
) -> Result<Paths, String> {
    let mut file = [0; PATH_CHARS];
    let mut key = [0; PATH_CHARS];
    let mut ipc = [0; PATH_CHARS];
    let mut file_bytes = (PATH_CHARS * 2) as u32;
    let mut key_bytes = file_bytes;
    let mut ipc_bytes = file_bytes;
    checked(
        "read the effective Sandboxie roots",
        query(
            file.as_mut_ptr(),
            key.as_mut_ptr(),
            ipc.as_mut_ptr(),
            &mut file_bytes,
            &mut key_bytes,
            &mut ipc_bytes,
        ),
    )?;
    Ok(Paths {
        file: text(&file)?,
        key: text(&key)?,
        ipc: text(&ipc)?,
    })
}

fn box_name(name: &str) -> Result<[u16; BOX_CHARS], String> {
    let suffix = name
        .strip_prefix("PF_")
        .ok_or("Expected a dedicated PortableForge box name.")?;
    if !(16..=29).contains(&suffix.len())
        || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err("Expected a fresh PortableForge box identifier.".into());
    }
    fixed(name)
}

fn validate_root(root: &Path) -> Result<&str, String> {
    let value = root
        .to_str()
        .ok_or("Choose a Unicode local box storage path.")?;
    let bytes = value.as_bytes();
    if bytes.len() < 4
        || !bytes[0].is_ascii_alphabetic()
        || &bytes[1..3] != b":\\"
        || value[3..]
            .split('\\')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || value.contains(['%', '\r', '\n', '\0', '"'])
        || value.encode_utf16().count() >= 240
    {
        return Err("Choose an ordinary local box storage path without variables.".into());
    }
    Ok(value)
}

fn same_file_root(actual: &str, expected: &str) -> Result<bool, String> {
    if actual
        .strip_prefix(r"\??\")
        .unwrap_or(actual)
        .eq_ignore_ascii_case(expected)
    {
        return Ok(true);
    }
    let drive = fixed::<3>(&expected[..2])?;
    let mut device = [0; PATH_CHARS];
    // SAFETY: validated drive name, bounded writable multi-string output.
    if unsafe { QueryDosDeviceW(drive.as_ptr(), device.as_mut_ptr(), device.len() as u32) } == 0 {
        return Err(os_error("resolve the box storage volume"));
    }
    Ok(actual.eq_ignore_ascii_case(&format!("{}{}", text(&device)?, &expected[2..])))
}

fn wide(value: &OsStr) -> Result<Vec<u16>, String> {
    let mut result: Vec<_> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err("A Windows path or argument contains an invalid character.".into());
    }
    result.push(0);
    Ok(result)
}

fn fixed<const N: usize>(value: &str) -> Result<[u16; N], String> {
    let utf16 = wide(OsStr::new(value))?;
    if utf16.len() > N {
        return Err("A Sandboxie setting or name is too long.".into());
    }
    let mut padded = [0; N];
    padded[..utf16.len()].copy_from_slice(&utf16);
    Ok(padded)
}

fn text(buffer: &[u16]) -> Result<String, String> {
    let end = buffer
        .iter()
        .position(|ch| *ch == 0)
        .ok_or("Sandboxie returned an unterminated string.")?;
    String::from_utf16(&buffer[..end]).map_err(|_| "Sandboxie returned malformed text.".into())
}

fn command_line(image: &OsStr, args: &[OsString]) -> Result<Vec<u16>, String> {
    fn quote(value: &OsStr, out: &mut Vec<u16>) -> Result<(), String> {
        let slash = b'\\' as u16;
        let quote = b'"' as u16;
        out.push(quote);
        let mut pending = 0;
        for ch in value.encode_wide() {
            if ch == 0 {
                return Err("A boxed process argument contains a NUL character.".into());
            }
            if ch == slash {
                pending += 1;
                continue;
            }
            let escaped = if ch == quote {
                pending * 2 + 1
            } else {
                pending
            };
            out.extend(std::iter::repeat(slash).take(escaped));
            pending = 0;
            out.push(ch);
        }
        out.extend(std::iter::repeat(slash).take(pending * 2));
        out.push(quote);
        Ok(())
    }
    let mut out = Vec::new();
    quote(image, &mut out)?;
    for arg in args {
        out.push(b' ' as u16);
        quote(arg, &mut out)?;
        if out.len() >= 32_767 {
            return Err("The boxed process command line is too long.".into());
        }
    }
    out.push(0);
    Ok(out)
}

fn current_session() -> Result<u32, String> {
    let mut session = 0;
    // SAFETY: output is writable and the PID is for this host process.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(os_error("read this Windows session"));
    }
    Ok(session)
}

fn service_running(name: &str) -> Result<(), String> {
    let service_name = wide(OsStr::new(name))?;
    // SAFETY: connect to the local SCM, requesting only connect/query access.
    let manager = unsafe { OpenSCManagerW(null(), null(), 1) };
    if manager.is_null() {
        return Err(os_error("open the local Windows service manager"));
    }
    let manager = ServiceHandle(manager);
    let service = unsafe { OpenServiceW(manager.0, service_name.as_ptr(), 4) };
    if service.is_null() {
        return Err(format!(
            "Sandboxie-Plus 1.18.8 x64 is required. {}",
            os_error(&format!("open service {name}"))
        ));
    }
    let service = ServiceHandle(service);
    let mut status = [0; 7]; // SERVICE_STATUS is seven DWORDs
                             // SAFETY: valid owned service handle and output matching SERVICE_STATUS.
    if unsafe { QueryServiceStatus(service.0, status.as_mut_ptr()) } == 0 {
        return Err(os_error(&format!("query service {name}")));
    }
    if status[1] != 4 {
        // SERVICE_RUNNING
        return Err(format!(
            "Sandboxie service {name} is installed but not running."
        ));
    }
    Ok(())
}

fn registry_key_exists(key: &str) -> Result<bool, String> {
    let key = wide(OsStr::new(key))?;
    let mut handle = null_mut();
    const HKEY_USERS: Handle = 0x80000003_u32 as i32 as isize as Handle;
    // SAFETY: exact driver-verified subkey under HKEY_USERS; only query access.
    let result = unsafe { RegOpenKeyExW(HKEY_USERS, key.as_ptr(), 0, 1, &mut handle) };
    match result {
        0 => {
            // SAFETY: RegOpenKeyExW just returned this handle successfully.
            unsafe { RegCloseKey(handle) };
            Ok(true)
        }
        2 | 3 => Ok(false), // ERROR_FILE_NOT_FOUND or ERROR_PATH_NOT_FOUND
        _ => Err(format!(
            "Could not verify the Sandboxie hive was unmounted: {}",
            io::Error::from_raw_os_error(result)
        )),
    }
}

fn reject_child(process: Handle, reason: String) -> String {
    // SAFETY: the exact service-created process handle is still retained.
    // A filtered handle may refuse termination. The coordinator must then
    // perform and verify scoped cleanup of the owned box.
    if unsafe { TerminateProcess(process, 1) } != 0 {
        reason
    } else {
        format!("{reason}\nThe retained handle could not stop the process. The owned box needs cleanup.")
    }
}

fn checked(action: &str, status: i32) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "Could not {action} (Sandboxie status 0x{:08x}).",
            status as u32
        ))
    }
}

fn os_error(action: &str) -> String {
    format!("Could not {action}: {}.", io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_command_keeps_spaces_quotes_and_trailing_slashes_in_one_argument() {
        let command = command_line(
            OsStr::new(r"C:\Program Files\Setup.exe"),
            &[
                OsString::from(""),
                OsString::from("a\"b"),
                OsString::from(r"C:\Folder With Spaces\"),
            ],
        )
        .unwrap();
        assert_eq!(
            text(&command).unwrap(),
            r#""C:\Program Files\Setup.exe" "" "a\"b" "C:\Folder With Spaces\\""#
        );
    }

    #[test]
    fn rejects_root_variables_parent_components_and_non_owned_box_names() {
        assert!(validate_root(Path::new(r"C:\Capture\PF_0123456789ABCDEF")).is_ok());
        assert!(validate_root(Path::new(r"C:\Capture\%USER%\box")).is_err());
        assert!(validate_root(Path::new(r"C:\Capture\..\box")).is_err());
        assert!(box_name("DefaultBox").is_err());
        assert!(box_name("PF_0123456789ABCDEF").is_ok());
    }
}
