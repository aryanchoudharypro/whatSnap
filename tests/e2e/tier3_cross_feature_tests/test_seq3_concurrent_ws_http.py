import os
import sys
import time
import subprocess
import tempfile
import threading
import unittest
import win32file

def create_ws_frame(payload_str):
	payload = payload_str.encode("utf-8")
	length = len(payload)
	mask = b"\x12\x34\x56\x78"
	masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
	if length <= 125:
		header = bytes([0x81, 0x80 | length]) + mask
	elif length <= 65535:
		header = bytes([0x81, 0x80 | 126]) + length.to_bytes(2, "big") + mask
	else:
		header = bytes([0x81, 0x80 | 127]) + length.to_bytes(8, "big") + mask
	return header + masked

def find_backend_executable():
	candidates = [
		os.path.abspath("backend/backend.exe"),
		os.path.abspath("target/debug/backend/backend.exe"),
		os.path.abspath("target/release/backend/backend.exe")
	]
	for path in candidates:
		if os.path.exists(path):
			return path
	raise FileNotFoundError("backend.exe executable not found in candidate paths")

def connect_named_pipe(pipe_name, timeout_sec=5.0):
	deadline = time.time() + timeout_sec
	while time.time() < deadline:
		try:
			return win32file.CreateFile(
				pipe_name,
				win32file.GENERIC_READ | win32file.GENERIC_WRITE,
				0,
				None,
				win32file.OPEN_EXISTING,
				0,
				None
			)
		except Exception as e:
			err_code = e.args[0] if e.args else None
			if err_code in (231, 2):
				try:
					win32pipe.WaitNamedPipe(pipe_name, 500)
				except Exception:
					time.sleep(0.05)
			else:
				time.sleep(0.05)
	raise TimeoutError(f"Timed out connecting to named pipe {pipe_name}")

class TestSeq3ConcurrentWsHttp(unittest.TestCase):
	def setUp(self):
		self.temp_dir = tempfile.mkdtemp(prefix="whantv_t3_seq3_")
		self.db_path = os.path.join(self.temp_dir, "seq3.db")
		self.pipe_name = r"\\.\pipe\whantv-ipc-seq3-test"
		self.backend_bin = find_backend_executable()

	def tearDown(self):
		if os.path.exists(self.temp_dir):
			try:
				for root, dirs, files in os.walk(self.temp_dir, topdown=False):
					for f in files:
						os.remove(os.path.join(root, f))
					for d in dirs:
						os.rmdir(os.path.join(root, d))
				os.rmdir(self.temp_dir)
			except Exception:
				pass

	def test_concurrent_ws_and_http_over_named_pipe(self):
		cmd = [self.backend_bin, "-db", self.db_path, "-pipe", self.pipe_name]
		proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

		h_init = connect_named_pipe(self.pipe_name)
		win32file.CloseHandle(h_init)

		ws_errors = []
		http_errors = []
		threads = []

		def ws_worker(thread_idx):
			try:
				for msg_idx in range(5):
					h = connect_named_pipe(self.pipe_name)
					upgrade_req = (
						b"GET /ws HTTP/1.1\r\n"
						b"Host: localhost\r\n"
						b"Upgrade: websocket\r\n"
						b"Connection: Upgrade\r\n"
						b"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
						b"Sec-WebSocket-Version: 13\r\n\r\n"
					)
					win32file.WriteFile(h, upgrade_req)
					err, resp = win32file.ReadFile(h, 4096)
					if err != 0 or b"101 Switching Protocols" not in resp:
						ws_errors.append(f"Thread {thread_idx} WS upgrade failed: {resp}")
						win32file.CloseHandle(h)
						return

					frame = create_ws_frame(f'{{"action":"ping","id":{msg_idx}}}')
					win32file.WriteFile(h, frame)
					time.sleep(0.05)
					win32file.CloseHandle(h)
			except Exception as e:
				ws_errors.append(f"Thread {thread_idx} exception: {e}")

		def http_worker(thread_idx):
			try:
				for _ in range(5):
					h = connect_named_pipe(self.pipe_name)
					req = b"GET /nonexistent HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n"
					win32file.WriteFile(h, req)
					err, resp = win32file.ReadFile(h, 4096)
					win32file.CloseHandle(h)
					if err != 0 or b"404 Not Found" not in resp:
						http_errors.append(f"Thread {thread_idx} HTTP 404 test failed: {resp}")
						return
					time.sleep(0.05)
			except Exception as e:
				http_errors.append(f"Thread {thread_idx} exception: {e}")

		for i in range(5):
			t_ws = threading.Thread(target=ws_worker, args=(i,))
			t_http = threading.Thread(target=http_worker, args=(i,))
			threads.append(t_ws)
			threads.append(t_http)

		for t in threads:
			t.start()

		for t in threads:
			t.join()

		self.assertEqual(len(ws_errors), 0, f"WS worker errors encountered: {ws_errors}")
		self.assertEqual(len(http_errors), 0, f"HTTP worker errors encountered: {http_errors}")

		h_sd = win32file.CreateFile(
			self.pipe_name,
			win32file.GENERIC_READ | win32file.GENERIC_WRITE,
			0,
			None,
			win32file.OPEN_EXISTING,
			0,
			None
		)
		win32file.WriteFile(h_sd, b"POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n")
		win32file.ReadFile(h_sd, 4096)
		win32file.CloseHandle(h_sd)

		exit_code = proc.wait(timeout=5)
		self.assertEqual(exit_code, 0, "Backend process did not exit with code 0 after concurrent test")

if __name__ == "__main__":
	unittest.main()
