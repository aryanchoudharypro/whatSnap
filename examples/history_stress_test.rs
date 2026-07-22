use std::{process::Command, time::Instant};
use winzapp_rust::ChatMessage;

fn run_stress_test_simulation() {
	let backend_dir = std::path::PathBuf::from("backend");
	let output = Command::new("go")
		.args(&["test", "-v", "./..."])
		.current_dir(&backend_dir)
		.output()
		.expect("Failed to execute backend go test");

	assert!(output.status.success(), "Backend Go stress test failed: {}", String::from_utf8_lossy(&output.stderr));

	let total_messages = 50000;
	let page_size = 50;
	let mut all_json_pages: Vec<Vec<String>> = Vec::new();

	for page in 0..(total_messages / page_size) {
		let mut current_page = Vec::new();
		for i in 0..page_size {
			let idx = (page * page_size) + i + 1;
			let json_str = format!(
				r#"{{"id":"msg_{}","text":"Synthetic chat message #{} payload","timestamp":{},"fromMe":{},"senderJid":"user_{}@s.whatsapp.net","senderName":"User {}"}}"#,
				idx, idx, 1700000000 + idx, idx % 2 == 0, idx % 50, idx % 50
			);
			current_page.push(json_str);
		}
		all_json_pages.push(current_page);
	}

	let mut loaded_history: Vec<ChatMessage> = Vec::new();
	let mut max_page_duration = std::time::Duration::from_secs(0);
	let mut total_duration = std::time::Duration::from_secs(0);

	for (page_idx, page) in all_json_pages.iter().enumerate() {
		let start = Instant::now();
		let mut page_msgs: Vec<ChatMessage> = Vec::with_capacity(page_size);

		for raw in page {
			let msg: ChatMessage = serde_json::from_str(raw).expect("Failed to parse ChatMessage JSON");
			let _formatted = msg.format_display(true);
			page_msgs.push(msg);
		}

		let elapsed = start.elapsed();
		total_duration += elapsed;
		if elapsed > max_page_duration {
			max_page_duration = elapsed;
		}

		assert!(elapsed < std::time::Duration::from_millis(10), "Page {} processing time {:?} exceeded 10ms limit", page_idx, elapsed);

		page_msgs.extend(loaded_history.drain(..));
		loaded_history = page_msgs;
	}

	assert_eq!(loaded_history.len(), total_messages, "Data truncation detected! Expected {} messages, got {}", total_messages, loaded_history.len());
	let avg_page_duration = total_duration / (all_json_pages.len() as u32);
	println!("Rust Stress Harness Passed: {} messages processed across {} pages.", loaded_history.len(), all_json_pages.len());
	println!("Page Processing Latency - Avg: {:?}, Max: {:?}", avg_page_duration, max_page_duration);
}

fn main() {
	run_stress_test_simulation();
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_history_stress() {
		run_stress_test_simulation();
	}
}
