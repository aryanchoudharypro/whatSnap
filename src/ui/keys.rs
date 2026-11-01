use std::{cell::Cell, rc::Rc};

use wxdragon::{event::KeyboardEvent, prelude::*};

pub fn is_enter(code: Option<i32>) -> bool {
	matches!(code, Some(WXK_RETURN | WXK_NUMPAD_ENTER))
}

pub fn is_escape(code: Option<i32>) -> bool {
	code == Some(WXK_ESCAPE)
}

pub fn is_space(code: Option<i32>) -> bool {
	code == Some(WXK_SPACE)
}

pub fn is_tab(code: Option<i32>) -> bool {
	code == Some(WXK_TAB)
}

pub fn is_left(code: Option<i32>) -> bool {
	matches!(code, Some(WXK_LEFT | WXK_NUMPAD_LEFT))
}

pub fn is_right(code: Option<i32>) -> bool {
	matches!(code, Some(WXK_RIGHT | WXK_NUMPAD_RIGHT))
}

pub fn is_up(code: Option<i32>) -> bool {
	matches!(code, Some(WXK_UP | WXK_NUMPAD_UP))
}

pub fn is_down(code: Option<i32>) -> bool {
	matches!(code, Some(WXK_DOWN | WXK_NUMPAD_DOWN))
}

pub fn is_home(code: Option<i32>) -> bool {
	matches!(code, Some(WXK_HOME | WXK_NUMPAD_HOME))
}

pub fn is_end(code: Option<i32>) -> bool {
	matches!(code, Some(WXK_END | WXK_NUMPAD_END))
}

pub fn is_letter(code: Option<i32>, letter: char) -> bool {
	code == Some(letter.to_ascii_uppercase() as i32)
}

pub fn is_context_menu_key(key: &KeyboardEvent) -> bool {
	if key.meta_down() {
		return false;
	}
	match key.get_key_code() {
		Some(WXK_WINDOWS_MENU) => true,
		Some(WXK_F10) => key.shift_down(),
		_ => false,
	}
}

pub fn on_enter<W: WindowEvents>(target: &W, release_handled: Rc<Cell<bool>>, action: impl Fn() + 'static) {
	let action = Rc::new(action);
	let action_down = action.clone();
	let handled_down = release_handled.clone();
	target.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if is_enter(key.get_key_code()) {
				handled_down.set(true);
				action_down();
				event.skip(false);
				return;
			}
			handled_down.set(false);
		}
		event.skip(true);
	});
	target.on_key_up(move |event| {
		if let WindowEventData::Keyboard(ref key) = event
			&& is_enter(key.get_key_code())
		{
			if !release_handled.replace(false) {
				action();
			}
			event.skip(false);
			return;
		}
		event.skip(true);
	});
}

pub fn escape_cancels(dialog: Dialog) -> impl FnMut(WindowEventData) + Copy + 'static {
	move |event: WindowEventData| {
		if let WindowEventData::Keyboard(ref key) = event
			&& is_escape(key.get_key_code())
		{
			dialog.end_modal(ID_CANCEL);
			event.skip(false);
			return;
		}
		event.skip(true);
	}
}

pub fn escape_cancels_anywhere(dialog: Dialog) {
	dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		escape_cancels(dialog)(WindowEventData::new(event));
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn numpad_twins_count_as_the_same_key() {
		assert!(is_enter(Some(WXK_NUMPAD_ENTER)));
		assert!(is_home(Some(WXK_NUMPAD_HOME)));
		assert!(is_end(Some(WXK_NUMPAD_END)));
		assert!(is_left(Some(WXK_NUMPAD_LEFT)));
		assert!(is_right(Some(WXK_NUMPAD_RIGHT)));
		assert!(is_up(Some(WXK_NUMPAD_UP)));
		assert!(is_down(Some(WXK_NUMPAD_DOWN)));
	}

	#[test]
	fn neighbouring_keys_are_not_confused() {
		assert!(!is_home(Some(WXK_NUMPAD_INSERT)));
		assert!(!is_end(Some(WXK_NUMPAD_DELETE)));
		assert!(!is_right(Some('\'' as i32)));
		assert!(!is_left(Some('%' as i32)));
	}

	#[test]
	fn letters_match_their_capital_code() {
		assert!(is_letter(Some('R' as i32), 'r'));
		assert!(is_letter(Some('R' as i32), 'R'));
		assert!(!is_letter(Some('E' as i32), 'r'));
		assert!(!is_letter(None, 'r'));
	}
}
