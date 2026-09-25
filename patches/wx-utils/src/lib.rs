#![warn(clippy::all, clippy::cargo, clippy::nursery, clippy::pedantic)]

mod about;
mod dialogs;
mod duration;
mod ids;
mod menu;
#[cfg(feature = "shortcuts")]
pub mod shortcuts;

pub use about::AboutBoxBuilder;
pub use dialogs::{
	DIALOG_PADDING, add_ok_cancel_footer, add_single_button_footer, add_yes_no_footer, bind_enter_confirms,
	build_ok_cancel_buttons, build_ok_cancel_buttons_on, build_yes_no_buttons, build_yes_no_buttons_on, confirm,
	dialog_padding, prompt_number, prompt_text, show_error, show_warning,
};
pub use duration::{format_duration_ms, format_duration_seconds};
pub use menu::{menu_label, set_menu_item_label};
