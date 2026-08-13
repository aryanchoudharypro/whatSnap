import os
import sys
import time
import subprocess
import tempfile
import unittest
import win32file

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

class TestF3ShutdownEndpoint(unittest.TestCase):
	def setUp(self):
		self.temp_dir = tempfile.mkdtemp(prefix="whantv_t1_f3_")
		self.db_path = os.path.join(self.temp_dir, "shutdown_test.db")
		self.pipe_name = r"\\.\pipe\whantv-ipc-f3-test"
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

	def test_shutdown_post_payload(self):
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

		shutdown_req = b"POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n"
		win32file.WriteFile(handle, shutdown_req)
		err, shutdown_resp = win32file.ReadFile(handle, 4096)
		self.assertEqual(err, 0)
		resp_str = shutdown_resp.decode("utf-8", errors="ignore")
		self.assertIn("HTTP/1.1 200 OK", resp_str)
		win32file.CloseHandle(handle)

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

	def test_shutdown_releases_db_file(self):
		cmd = [self.backend_bin, "-db", self.db_path, "-pipe", self.pipe_name]
		proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

		handle = None
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
				break
			except Exception:
				time.sleep(0.1)

		shutdown_req = b"POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n"
		win32file.WriteFile(handle, shutdown_req)
		win32file.CloseHandle(handle)

		proc.wait(timeout=5)
		time.sleep(0.2)

		can_remove_db = False
		try:
			if os.path.exists(self.db_path):
				os.remove(self.db_path)
			can_remove_db = True
		except Exception:
			can_remove_db = False

		self.assertTrue(can_remove_db, "DB file remained locked after shutdown")

if __name__ == "__main__":
	unittest.main()
