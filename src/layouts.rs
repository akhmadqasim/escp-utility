//! Generated from phase2/layout/LAYOUT.json (printing_preferences.subdialogs)
//! by gen_layouts.py: sub-dialogs shown as faithful but disabled layouts.
//! Rectangles are px at 96 dpi; overlapping model alternatives keep one.

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
    size: (246, 244),
    reason: "Page order is not mapped yet: CUPS number-up-layout is close, but which Windows picture is which order is not traced.",
    controls: &[
        (K::Frame, 47, 63, 42, 46, ""),
        (K::Frame, 156, 63, 42, 46, ""),
        (K::Frame, 47, 127, 42, 46, ""),
        (K::Frame, 156, 127, 42, 46, ""),
        (K::Check, 18, 189, 195, 13, "Contiguously Layout XPS Documents"),
        (K::Button, 11, 213, 69, 23, "OK"),
        (K::Button, 87, 213, 69, 23, "Cancel"),
        (K::Button, 164, 213, 69, 23, "Help"),
        (K::Text, 12, 15, 224, 44, "Select the layout method you want to use, and click OK."),
    ],
};

pub const POSTER: Layout = Layout {
    caption: "Poster Settings",
    size: (338, 379),
    reason: "No Linux option: the L800 driver prints posters without cutting guides or panel selection.",
    controls: &[
        (K::Button, 29, 343, 69, 23, "OK"),
        (K::Button, 134, 343, 69, 23, "Cancel"),
        (K::Button, 239, 343, 69, 23, "Help"),
        (K::Check, 17, 288, 113, 18, "Print Cutting Guides"),
        (K::Radio, 38, 312, 164, 18, "Overlapping Alignment Marks"),
        (K::Radio, 219, 312, 75, 18, "Trim Lines"),
        (K::Frame, 143, 7, 182, 273, ""),
        (K::Text, 6, 13, 129, 67, "Please select the panels that you don't wish to be printed. (The panels which appear grayed out will not be printed)"),
    ],
};

pub const MARGINS: Layout = Layout {
    caption: "Margins",
    size: (285, 226),
    reason: "No Linux option: the L800 CUPS driver has no 2-sided printing.",
    controls: &[
        (K::Text, 12, 13, 68, 16, "Binding Edge"),
        (K::Radio, 89, 13, 48, 16, "Left"),
        (K::Radio, 150, 13, 48, 16, "Top"),
        (K::Radio, 212, 13, 48, 16, "Right"),
        (K::Frame, 99, 34, 30, 33, ""),
        (K::Frame, 161, 34, 30, 33, ""),
        (K::Frame, 224, 34, 30, 33, ""),
        (K::Text, 12, 89, 74, 16, "Binding Margin"),
        (K::Edit, 107, 85, 53, 20, ""),
        (K::UpDown, 144, 85, 12, 20, ""),
        (K::Text, 170, 89, 90, 15, ""),
        (K::Group, 53, 109, 182, 37, "Unit"),
        (K::Radio, 62, 124, 75, 16, "0.01 cm"),
        (K::Radio, 152, 124, 75, 16, "0.01 inch"),
        (K::Check, 21, 171, 189, 13, "Contiguously Bind XPS Documents"),
        (K::Button, 18, 195, 69, 23, "OK"),
        (K::Button, 108, 195, 69, 23, "Cancel"),
        (K::Button, 198, 195, 69, 23, "Help"),
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
        (K::Frame, 21, 10, 155, 206, ""),
        (K::Text, 213, 16, 68, 20, "Watermark"),
        (K::Combo, 282, 11, 135, 21, ""),
        (K::Text, 213, 49, 68, 20, "Text"),
        (K::Edit, 282, 47, 135, 20, ""),
        (K::Button, 423, 46, 69, 23, "Browse"),
        (K::Line, 213, 78, 279, 3, ""),
        (K::Text, 213, 99, 68, 20, "Color"),
        (K::Combo, 282, 94, 135, 21, ""),
        (K::Group, 213, 132, 279, 156, "Font Setting"),
        (K::Text, 231, 154, 50, 20, "Font"),
        (K::Combo, 282, 148, 135, 21, ""),
        (K::Text, 231, 192, 50, 20, "Style"),
        (K::List, 282, 180, 135, 96, ""),
        (K::Group, 213, 296, 279, 101, "Rotation"),
        (K::Text, 231, 320, 32, 20, "Angle"),
        (K::Edit, 279, 319, 36, 20, ""),
        (K::Frame, 269, 319, 11, 20, ""),
        (K::Text, 320, 322, 38, 20, "Degree"),
        (K::CText, 357, 307, 18, 20, "-"),
        (K::CText, 468, 307, 18, 20, "+"),
        (K::Slider, 356, 322, 129, 20, "Slider2"),
        (K::Frame, 279, 345, 36, 39, ""),
        (K::Text, 330, 361, 105, 20, "Rotate with Mouse"),
        (K::Text, 15, 241, 56, 16, "Position"),
        (K::Combo, 72, 236, 123, 21, ""),
        (K::Text, 15, 284, 56, 16, "Density"),
        (K::Text, 68, 267, 35, 13, "Light"),
        (K::Text, 173, 267, 30, 13, "Dark"),
        (K::Slider, 72, 280, 123, 33, "Slider3"),
        (K::Text, 15, 325, 56, 16, "Size"),
        (K::CText, 75, 317, 18, 20, "-"),
        (K::CText, 179, 317, 18, 20, "+"),
        (K::Slider, 72, 328, 123, 33, "Slider5"),
        (K::Text, 15, 377, 56, 16, "Layering"),
        (K::Combo, 72, 372, 123, 21, ""),
        (K::Button, 258, 406, 69, 23, "OK"),
        (K::Button, 341, 406, 69, 23, "Cancel"),
        (K::Button, 423, 406, 69, 23, "Help"),
    ],
};
