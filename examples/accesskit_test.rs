use accesskit::{ActionHandler, ActionRequest, ActivationHandler, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use accesskit_windows::SubclassingAdapter;
use windows::Win32::Foundation::HWND;
use wxdragon::prelude::*;

fn main() {
	let _ = wxdragon::main(|_| {
		let frame = Frame::builder().with_title("AccessKit Test").with_size(Size::new(400, 300)).build();

		let panel = Panel::builder(&frame).build();
		let hwnd = HWND(panel.handle_ptr() as *mut std::ffi::c_void);

		let _adapter = SubclassingAdapter::new(hwnd, NullHandler, NullHandler);

		frame.show(true);
	});
}

struct NullHandler;

impl ActionHandler for NullHandler {
	fn do_action(&mut self, _request: ActionRequest) {}
}

impl ActivationHandler for NullHandler {
	fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
		let root = Node::new(Role::List);
		Some(TreeUpdate {
			nodes: vec![(NodeId(1), root)],
			tree: Some(Tree::new(NodeId(1))),
			focus: NodeId(1),
			tree_id: TreeId(accesskit::Uuid::nil()),
		})
	}
}
