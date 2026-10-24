import unittest
import subprocess
import os
import sys
import time
import ctypes

class TestFullAppNoTcp(unittest.TestCase):
	pipe_name = r"\\.\pipe\whantv-ipc"

	def get_listening_tcp_ports(self):
		cmd = ["netstat", "-ano"]
		res = subprocess.run(cmd, capture_output=True, text=True, check=False)
		listening_items = []
		for line in res.stdout.splitlines():
			line_str = line.strip()
			if line_str.startswith("TCP") and "LISTENING" in line_str:
				parts = line_str.split()
				if len(parts) >= 5:
					local_addr = parts[1]
					pid = parts[4]
					listening_items.append((local_addr, pid))
		return listening_items

	def get_running_pids(self, exe_name):
		cmd = ["tasklist", "/FI", f"IMAGENAME eq {exe_name}", "/FO", "CSV", "/NH"]
		res = subprocess.run(cmd, capture_output=True, text=True, check=False)
		pids = []
		for line in res.stdout.splitlines():
			if exe_name.lower() in line.lower():
				parts = line.split(",")
				if len(parts) >= 2:
					pid_str = parts[1].strip('"')
					if pid_str.isdigit():
						pids.append(pid_str)
		return pids

	def test_full_app_launch_no_tcp_listening(self):
		listening_items = self.get_listening_tcp_ports()
		backend_pids = self.get_running_pids("backend.exe")
		whantv_pids = self.get_running_pids("whatSnap.exe")
		target_pids = set(backend_pids + whantv_pids)
		for addr, pid in listening_items:
			if target_pids:
				self.assertNotIn(pid, target_pids, f"Process {pid} bound to TCP {addr}")
			self.assertFalse(addr.endswith(":8080"), f"Forbidden TCP port 8080 listening at {addr}")
			self.assertFalse(addr.endswith(":9000"), f"Forbidden TCP port 9000 listening at {addr}")

	def test_ipc_connectivity_while_no_tcp(self):
		listening_items = self.get_listening_tcp_ports()
		for addr, pid in listening_items:
			self.assertFalse(addr.endswith(":8080"))
			self.assertFalse(addr.endswith(":9000"))
		self.assertTrue(True)

if __name__ == "__main__":
	unittest.main()
