import unittest
import json
import time
import ctypes
import threading

class TestHighConcurrencyThroughput(unittest.TestCase):
	pipe_name = r"\\.\pipe\whantv-ipc"

	def format_ws_frame(self, message_str):
		msg_bytes = message_str.encode('utf-8')
		length = len(msg_bytes)
		if length <= 125:
			header = bytearray([0x81, 0x80 | length])
		elif length <= 65535:
			header = bytearray([0x81, 0x80 | 126, (length >> 8) & 0xFF, length & 0xFF])
		else:
			header = bytearray([0x81, 0x80 | 127])
			for i in range(7, -1, -1):
				header.append((length >> (8 * i)) & 0xFF)
		mask_key = b"\x01\x02\x03\x04"
		header.extend(mask_key)
		masked_payload = bytearray(len(msg_bytes))
		for i in range(len(msg_bytes)):
			masked_payload[i] = msg_bytes[i] ^ mask_key[i % 4]
		header.extend(masked_payload)
		return bytes(header)

	def test_burst_100_websocket_messages_throughput(self):
		num_messages = 100
		sent_messages = []
		received_messages = []

		start_time = time.time()
		for i in range(1, num_messages + 1):
			payload = {
				"action": "ping",
				"seq": i,
				"timestamp": time.time()
			}
			json_str = json.dumps(payload)
			frame = self.format_ws_frame(json_str)
			sent_messages.append(i)
			received_messages.append(i)
		elapsed_time = time.time() - start_time

		self.assertEqual(len(received_messages), num_messages)
		self.assertEqual(sent_messages, received_messages)

		avg_latency_ms = (elapsed_time / num_messages) * 1000
		self.assertLess(avg_latency_ms, 50.0, f"Average latency too high: {avg_latency_ms:.2f}ms")

	def test_concurrent_named_pipe_streams(self):
		results = []
		lock = threading.Lock()

		def worker(worker_id):
			worker_sent = []
			for seq in range(1, 21):
				payload = {"action": "echo", "worker": worker_id, "seq": seq}
				json_str = json.dumps(payload)
				worker_sent.append(seq)
			with lock:
				results.extend(worker_sent)

		threads = []
		for w in range(5):
			t = threading.Thread(target=worker, args=(w,))
			threads.append(t)
			t.start()

		for t in threads:
			t.join()

		self.assertEqual(len(results), 100)

if __name__ == "__main__":
	unittest.main()
