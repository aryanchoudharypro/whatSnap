//! Menu labels that carry a keyboard shortcut.

use wxdragon::prelude::*;

/// Joins a menu item's text and its shortcut into the `"Open\tCtrl+O"` form wx wants, where the
/// tab is what makes wx right-align the shortcut in the menu.
///
/// An empty `shortcut` gives back a bare label, so an unbound action does not get a trailing tab
/// and a blank column.
#[must_use]
pub fn menu_label(base: &str, shortcut: &str) -> String {
	if shortcut.is_empty() { base.to_string() } else { format!("{base}\t{shortcut}") }
}

/// Relabels an existing menu item, keeping its shortcut column in step.
///
/// Does nothing when `id` is not in `menu_bar`, so a menu that varies by platform or build can
/// be relabelled in one pass without every caller checking first.
pub fn set_menu_item_label(menu_bar: &MenuBar, id: i32, base: &str, shortcut: &str) {
	if let Some(item) = menu_bar.find_item(id) {
		item.set_label(&menu_label(base, shortcut));
	}
}

#[cfg(test)]
mod tests {
	use super::menu_label;

	#[test]
	fn a_shortcut_is_tab_separated() {
		assert_eq!(menu_label("&Open", "Ctrl+O"), "&Open\tCtrl+O");
	}

	#[test]
	fn no_shortcut_leaves_a_bare_label() {
		assert_eq!(menu_label("&Open", ""), "&Open");
	}
}
