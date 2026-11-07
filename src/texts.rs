//! The dialog texts: our own wording, with the meaning of the Windows
//! string named in each comment. Shared with examples/screenshot.rs.

/// Meaning of E_IHSRG5P 4210.
pub const NOZZLE_INTRO: &str = "Before you print the nozzle check pattern, make sure the printer is switched on and connected to this computer.\n\nTake out the CD/DVD tray and set the front tray to the paper position; otherwise the nozzle check cannot run. Load A4 or Letter plain paper in the sheet feeder.\n\nClick [Print] to print the pattern, or [Cancel] to close.";
/// Meaning of E_IHSRG5P 4211.
pub const NOZZLE_RESULT: &str = "Compare your printout with the samples above. If lines are missing or broken, click [Clean] to clean the print head. If the pattern is complete, click [Finish].";
/// Meaning of E_IHSRG5P 4200 / 4201 (same text).
pub const CLEAN_INTRO: &str = "First take out the CD/DVD tray and set the front tray to the paper position; otherwise head cleaning cannot run. Select [Cleaning], then click [Start] to clean the print head nozzles.\n\nCleaning uses ink, so clean only when printouts show faint areas or gaps. Run a Nozzle Check first to see whether cleaning is needed.\n\nNote: select [Power Cleaning] only if print quality has not improved after three normal cleanings. Power Cleaning uses more ink than a normal cleaning.\n\nCaution: keep the CD/DVD tray out until cleaning has finished.";
/// Meaning of E_IHSRG5P 10553.
pub const POWER_CONFIRM: &str = "Do you really want to run Power Cleaning? It uses more ink than a normal cleaning.\n\nBefore you start, check that the transportation lock is in the unlocked (Printing) position, as in the picture below.";
/// Meaning of E_IHSRG5P 4203.
pub const CLEANING_NOW: &str = "The print head is being cleaned.\nPlease wait.\nCaution: keep the CD/DVD tray out until cleaning has finished.";
/// Meaning of E_IHSRG5P 4204 (status path).
pub const CLEAN_DONE_STATUS: &str = "The cleaning cycle has finished.\n\nTo check that it worked, click [Print Nozzle Check Pattern].";
/// Meaning of E_IHSRG5P 4205 (no-status path).
pub const CLEAN_DONE: &str = "Cleaning is finished when the power light on the printer stops flashing.\n\nThen click [Print Nozzle Check Pattern] to check that the cleaning worked.";
/// Meaning of E_IHSRG5P 3000.
pub const COMM_ERROR: &str = "The printer cannot be reached. Check that it is switched on and that the cable is connected. If a nozzle check pattern is printing, switch the printer off and on again to reset it.";
/// Meaning of E_IHSRG5P 4220.
pub const PAPER_GUIDE: &str = "Plain paper is used to clean the inside of the printer.\n\nLoad A4 or Letter plain paper in the printer, then click [Cleaning].\nRepeat this until the inside of the printer is clean.";
/// Meaning of E_IHSRG5P 1133.
pub const SEE_DETAILS: &str = "Click [Details] to see more.";
pub const DRY_RUN_NOTE: &str = "Dry run: no job was sent to the printer.\n\n";
