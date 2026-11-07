//! Generated from phase2/layout/LAYOUT.json (printing_preferences.subdialogs)
//! by gen_layouts.py: sub-dialogs shown as faithful but disabled layouts.
//! Rectangles are px at 96 dpi; overlapping model alternatives keep one.
//! Print Layout, Poster Settings, Margins and Watermark were then re-spaced
//! by hand per phase2/layout/TIDY-dialogs.md (8 px margins, 75x23 buttons
//! right-aligned; Print Layout is 7 px wider so its three buttons fit).

use crate::LayoutKind as K;

/// (kind, x, y, w, h, text)
pub type Ctl = (K, i32, i32, i32, i32, &'static str);

pub struct Layout {
    pub caption: &'static str,
    pub size: (i32, i32),
    /// Tooltip on every control: why it is off.
    pub reason: &'static str,
    pub controls: &'static [Ctl],
}

pub const PAGE_ORDER: Layout = Layout {
    caption: "Print Layout",
    size: (253, 244),
    reason: "Page order is not mapped yet: CUPS number-up-layout is close, but which Windows picture is which order is not traced.",
    controls: &[
        (K::Text, 8, 8, 237, 40, "Select the layout method you want to use, and click OK."),
        (K::Frame, 79, 56, 42, 46, ""),
        (K::Frame, 133, 56, 42, 46, ""),
        (K::Frame, 79, 108, 42, 46, ""),
        (K::Frame, 133, 108, 42, 46, ""),
        (K::Check, 8, 189, 237, 16, "Contiguously Layout XPS Documents"),
        (K::Button, 8, 213, 75, 23, "OK"),
        (K::Button, 89, 213, 75, 23, "Cancel"),
        (K::Button, 170, 213, 75, 23, "Help"),
    ],
};

pub const POSTER: Layout = Layout {
    caption: "Poster Settings",
    size: (338, 379),
    reason: "No Linux option: the L800 driver prints posters without cutting guides or panel selection.",
    controls: &[
        (K::Text, 8, 8, 127, 67, "Please select the panels that you don't wish to be printed. (The panels which appear grayed out will not be printed)"),
        (K::Frame, 143, 8, 187, 272, ""),
        (K::Check, 8, 288, 322, 16, "Print Cutting Guides"),
        (K::Radio, 24, 310, 170, 16, "Overlapping Alignment Marks"),
        (K::Radio, 200, 310, 100, 16, "Trim Lines"),
        (K::Button, 93, 348, 75, 23, "OK"),
        (K::Button, 174, 348, 75, 23, "Cancel"),
        (K::Button, 255, 348, 75, 23, "Help"),
    ],
};

pub const MARGINS: Layout = Layout {
    caption: "Margins",
    size: (285, 210),
    reason: "No Linux option: the L800 CUPS driver has no 2-sided printing.",
    controls: &[
        (K::Text, 8, 8, 68, 16, "Binding Edge"),
        (K::Radio, 89, 8, 48, 16, "Left"),
        (K::Radio, 150, 8, 48, 16, "Top"),
        (K::Radio, 212, 8, 48, 16, "Right"),
        (K::Frame, 99, 30, 30, 33, ""),
        (K::Frame, 161, 30, 30, 33, ""),
        (K::Frame, 224, 30, 30, 33, ""),
        (K::Text, 8, 75, 74, 16, "Binding Margin"),
        (K::Edit, 107, 71, 53, 20, ""),
        (K::UpDown, 144, 71, 12, 20, ""),
        (K::Text, 170, 75, 90, 15, ""),
        (K::Group, 8, 99, 269, 40, "Unit"),
        (K::Radio, 16, 117, 75, 16, "0.01 cm"),
        (K::Radio, 152, 117, 75, 16, "0.01 inch"),
        (K::Check, 8, 147, 269, 16, "Contiguously Bind XPS Documents"),
        (K::Button, 40, 179, 75, 23, "OK"),
        (K::Button, 121, 179, 75, 23, "Cancel"),
        (K::Button, 202, 179, 75, 23, "Help"),
    ],
};

pub const CUSTOM_SETTINGS: Layout = Layout {
    caption: "Custom Settings",
    size: (626, 471),
    reason: "Windows-only: named custom settings. On Linux, lpoptions instances are the closest thing.",
    controls: &[
        (K::Text, 17, 179, 56, 15, "Name"),
        (K::Edit, 75, 176, 158, 20, ""),
        (K::Text, 17, 202, 56, 13, "Icon"),
        (K::List, 15, 218, 216, 54, ""),
        (K::Text, 17, 278, 116, 13, "Comments (optional)"),
        (K::Edit, 18, 296, 216, 85, ""),
        (K::Text, 17, 385, 116, 13, "Creation Date and Time"),
        (K::Frame, 18, 403, 216, 20, ""),
        (K::Button, 299, 439, 69, 23, "Save"),
        (K::Button, 381, 439, 69, 23, "Delete"),
        (K::Button, 464, 439, 69, 23, "Close"),
        (K::Button, 546, 439, 69, 23, "Help"),
        (K::List, 248, 23, 366, 401, ""),
        (K::Text, 17, 7, 98, 13, "List"),
        (K::List, 18, 23, 216, 137, ""),
    ],
};

pub const WATERMARK: Layout = Layout {
    caption: "Watermark",
    size: (510, 440),
    reason: "Off: the installed L800 driver points at watermark files that do not exist, so watermarks would not print.",
    controls: &[
        // Left column: preview, then rows 46 px apart (labels x 8, controls x 72).
        (K::Frame, 24, 8, 155, 206, ""),
        (K::Text, 8, 232, 56, 16, "Position"),
        (K::Combo, 72, 228, 123, 21, ""),
        (K::Text, 8, 278, 56, 16, "Density"),
        (K::Text, 68, 262, 35, 13, "Light"),
        (K::Text, 173, 262, 30, 13, "Dark"),
        (K::Slider, 72, 274, 123, 33, "Slider3"),
        (K::Text, 8, 324, 56, 16, "Size"),
        (K::CText, 75, 308, 18, 20, "-"),
        (K::CText, 179, 308, 18, 20, "+"),
        (K::Slider, 72, 320, 123, 33, "Slider5"),
        (K::Text, 8, 370, 56, 16, "Layering"),
        (K::Combo, 72, 366, 123, 21, ""),
        // Right column x 213..502.
        (K::Text, 213, 12, 68, 20, "Watermark"),
        (K::Combo, 282, 8, 135, 21, ""),
        (K::Text, 213, 39, 68, 20, "Text"),
        (K::Edit, 282, 35, 135, 20, ""),
        (K::Button, 427, 34, 75, 23, "Browse"),
        (K::Line, 213, 64, 289, 3, ""),
        (K::Text, 213, 76, 68, 20, "Color"),
        (K::Combo, 282, 72, 135, 21, ""),
        (K::Group, 213, 99, 289, 150, "Font Setting"),
        (K::Text, 231, 123, 50, 20, "Font"),
        (K::Combo, 282, 119, 135, 21, ""),
        (K::Text, 231, 148, 50, 20, "Style"),
        (K::List, 282, 146, 135, 95, ""),
        (K::Group, 213, 255, 289, 96, "Rotation"),
        (K::Text, 231, 279, 32, 20, "Angle"),
        (K::Edit, 279, 275, 36, 20, ""),
        (K::Frame, 269, 275, 11, 20, ""),
        (K::Text, 320, 279, 38, 20, "Degree"),
        (K::CText, 357, 264, 18, 20, "-"),
        (K::CText, 477, 264, 18, 20, "+"),
        (K::Slider, 356, 276, 138, 20, "Slider2"),
        (K::Frame, 279, 302, 36, 39, ""),
        (K::Text, 330, 316, 105, 20, "Rotate with Mouse"),
        (K::Button, 265, 409, 75, 23, "OK"),
        (K::Button, 346, 409, 75, 23, "Cancel"),
        (K::Button, 427, 409, 75, 23, "Help"),
    ],
};
