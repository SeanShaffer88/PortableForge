//! Primary wording for packaging an unpacked app folder.

pub const PICK_TITLE: &str = "Choose an App";
pub const PICK_HELP: &str = "Choose a setup or program files. You can also drop them here.";
pub const CHOOSE_FOLDER: &str = "Choose App Folder…";
pub const CHOOSE_FILES: &str = "Choose App Files…";
pub const CHOOSE_SOURCE: &str = "Choose an App Source";
pub const CHANGE_SOURCE: &str = "Change Source";
pub const CHECKING_SOURCE: &str = "Checking Your Source";
pub const CHECKING_SELECTION: &str = "Checking the selected files…";
pub const STAGING_SOURCE: &str = "Preparing the app files…";
pub const STOP_INTAKE: &str = "Cancel Preparation";
pub const INPUT_NOT_READY: &str =
    "Choose one ZIP, one app folder, or files for one app from the same folder.";
pub const SAVE_DROPPED_FILES: &str = "Save every dropped file to a local folder and try again.";
pub const SOURCE_STOPPING: &str = "Stopping and cleaning up the app files…";
pub const SOURCE_STOPPED: &str = "App file preparation stopped. Choose a source to try again.";
pub const CLEANING_SOURCE: &str = "Removing temporary app files…";
pub const CLEANING_SOURCE_TITLE: &str = "Cleaning Up App Files";
pub const SOURCE_CLEANUP_FAILED: &str =
    "Temporary app files could not be removed. Open Show Details for the location.";
pub const HEADER: &str = "Build a Portable App";
pub const REVIEW: &str = "Review Your App";
pub const CHOOSE_PROGRAM: &str = "Choose the File That Starts the App";
pub const PREPARED: &str = "Choose the Original App Folder";
pub const PREPARED_HELP: &str =
    "This is already a PortableApps package. Choose the original unpacked app folder.";
pub const SCANNING: &str = "Looking for the file that starts the app…";
pub const READING_PROGRAM: &str = "Reading the selected file’s details…";
pub const BUILD: &str = "Build Installer";
pub const BUILDING: &str = "Building Your Installer";
pub const BUILT: &str = "Installer Built";
pub const STOP: &str = "Stop After This Step";
pub const STOPPING: &str = "Stopping after the current step…";
pub const STOPPED: &str = "Build stopped before the installer was saved.";
pub const RIGHTS: &str = "I have permission to include these app files.";
pub const UNVERIFIED: &str = "App behavior after moving it is unverified.";
pub const TEST_INSTALLER: &str = "Test Installer…";
pub const TEST_CONFIRM: &str = "Test the Built Installer";
pub const TEST_TRUST_HELP: &str = "Continue only if you trust the app and its files.";
pub const TEST_SETUP_HELP: &str =
    "Complete setup in a fresh Sandboxie-Plus box. Leave any option to open the app unchecked.";
pub const TEST_LAUNCH_HELP: &str =
    "Change a setting on the first launch, check it on the second, and close the app each time.";
pub const TEST_START: &str = "Start Test";
pub const TEST_RUNNING: &str = "Testing Your Installer";
pub const TEST_CHECKING: &str = "Checking the installer and build receipt…";
pub const TEST_USE_WINDOWS: &str = "Follow the installer and app windows.";
pub const TEST_CANCEL: &str = "Cancel Test";
pub const TEST_STOPPING: &str = "Stopping the test and cleaning up the box…";
pub const TEST_FINISHED: &str = "Installer Test Finished";
pub const TEST_FINISHED_HELP: &str =
    "Setup and both app launches completed. The test box was removed.";
pub const TEST_CANCELLED: &str = "Installer Test Cancelled";
pub const TEST_CANCELLED_HELP: &str = "The built installer is still available.";
pub const TEST_FAILED: &str = "Installer Test Could Not Finish";
pub const TEST_IDENTITY_HELP: &str =
    "The installer or receipt could not be verified. Build a new installer to test it.";
pub const TEST_STARTUP_HELP: &str =
    "The Sandboxie-Plus test could not start. The built installer is still available.";
pub const TEST_FAILURE_HELP: &str =
    "Open Show Test Details to see what happened. The built installer is still available.";
pub const TEST_CLEANUP_HELP: &str =
    "Could not confirm test box cleanup. Open Show Test Details for recovery information.";
pub const TEST_DETAILS: &str = "Show Test Details";
