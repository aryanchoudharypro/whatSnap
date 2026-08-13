import os
import sys
import time
import subprocess
import tempfile
import unittest
import win32file
import win32pipe

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

class TestSeq1FullLifecycle(unittest.TestCase):
	def setUp(self):
		self.temp_dir = tempfile.mkdtemp(prefix="whantv_t3_seq1_")
		self.db_path = os.path.join(self.temp_dir, "seq1.db")
		self.pipe_name = r"\\.\pipe\whantv-ipc-seq1-test"
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

	def test_full_lifecycle(self):
		cmd = [self.backend_bin, "-db", self.db_path, "-pipe", self.pipe_name]
		proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

		handle = None
		connected = False
		for _ in range(50):
			try:
				handle = win32file.CreateFile(
					self.pipe_name,
					win32file.GENERIC_READ | win32file.GENERIC_WRITE,
					0,
					None,
					win32file.OPEN_EXISTING,
					0,
					None
				)
				connected = True
				break
			except Exception:
				time.sleep(0.1)

		self.assertTrue(connected, "Failed to connect to backend named pipe listener")

		ws_upgrade = (
			b"GET /ws HTTP/1.1\r\n"
			b"Host: localhost\r\n"
			b"Upgrade: websocket\r\n"
			b"Connection: Upgrade\r\n"
			b"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
			b"Sec-WebSocket-Version: 13\r\n\r\n"
		)
		win32file.WriteFile(handle, ws_upgrade)
		err, resp_bytes = win32file.ReadFile(handle, 4096)
		self.assertEqual(err, 0)
		resp_str = resp_bytes.decode("utf-8", errors="ignore")
		self.assertIn("HTTP/1.1 101 Switching Protocols", resp_str)
		self.assertIn("Upgrade: websocket", resp_str)

		frame = create_ws_frame('{"action":"ping"}')
		win32file.WriteFile(handle, frame)
		time.sleep(0.2)
		win32file.CloseHandle(handle)

		handle_shutdown = win32file.CreateFile(
			self.pipe_name,
			win32file.GENERIC_READ | win32file.GENERIC_WRITE,
			0,
			None,
			win32file.OPEN_EXISTING,
			0,
			None
		)
		shutdown_req = b"POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n"
		win32file.WriteFile(handle_shutdown, shutdown_req)
		err, shutdown_resp = win32file.ReadFile(handle_shutdown, 4096)
		self.assertEqual(err, 0)
		self.assertIn("HTTP/1.1 200 OK", shutdown_resp.decode("utf-8", errors="ignore"))
		win32file.CloseHandle(handle_shutdown)

		exit_code = proc.wait(timeout=5)
		self.assertEqual(exit_code, 0, "Backend process did not exit with code 0")

		pipe_released = False
		for _ in range(30):
			try:
				h = win32file.CreateFile(
					self.pipe_name,
					win32file.GENERIC_READ | win32file.GENERIC_WRITE,
					0,
					None,
					win32file.OPEN_EXISTING,
					0,
					None
				)
				win32file.CloseHandle(h)
				time.sleep(0.1)
			except Exception:
				pipe_released = True
				break

		self.assertTrue(pipe_released, "Named pipe was not freed after process shutdown")

if __name__ == "__main__":
	unittest.main()
