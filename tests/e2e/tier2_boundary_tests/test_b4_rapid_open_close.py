import unittest
import ctypes
from ctypes import wintypes
import os
import sys
import time
import uuid
import threading

kernel32 = ctypes.windll.kernel32

INVALID_HANDLE_VALUE = wintypes.HANDLE(-1).value
GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
OPEN_EXISTING = 3
FILE_ATTRIBUTE_NORMAL = 0x80

PIPE_ACCESS_DUPLEX = 0x00000003
PIPE_TYPE_BYTE = 0x00000000
PIPE_READMODE_BYTE = 0x00000000
PIPE_WAIT = 0x00000000
PIPE_UNLIMITED_INSTANCES = 255

ERROR_BROKEN_PIPE = 109

kernel32.CreateNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID]
kernel32.CreateNamedPipeW.restype = wintypes.HANDLE

kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
kernel32.CreateFileW.restype = wintypes.HANDLE

kernel32.ReadFile.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
kernel32.ReadFile.restype = wintypes.BOOL

kernel32.WriteFile.argtypes = [wintypes.HANDLE, wintypes.LPCVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
kernel32.WriteFile.restype = wintypes.BOOL

kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL

kernel32.ConnectNamedPipe.argtypes = [wintypes.HANDLE, wintypes.LPVOID]
kernel32.ConnectNamedPipe.restype = wintypes.BOOL

kernel32.DisconnectNamedPipe.argtypes = [wintypes.HANDLE]
kernel32.DisconnectNamedPipe.restype = wintypes.BOOL

kernel32.GetProcessHandleCount.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
kernel32.GetProcessHandleCount.restype = wintypes.BOOL

kernel32.GetCurrentProcess.argtypes = []
kernel32.GetCurrentProcess.restype = wintypes.HANDLE

kernel32.GetLastError.argtypes = []
kernel32.GetLastError.restype = wintypes.DWORD

class TestRapidOpenClose(unittest.TestCase):

	def test_rapid_connection_disconnection_stress(self):
		pipe_name = r"\\.\pipe\whantv_rapid_stress_" + str(uuid.uuid4())[:8]
		iterations = 50
		server = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		self.assertNotEqual(server, INVALID_HANDLE_VALUE)

		successes = 0
		for _ in range(iterations):
			t = threading.Thread(target=lambda: kernel32.ConnectNamedPipe(server, None))
			t.start()
			client = kernel32.CreateFileW(
				pipe_name,
				GENERIC_READ | GENERIC_WRITE,
				0,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None
			)
			t.join(timeout=0.5)
			if client != INVALID_HANDLE_VALUE:
				successes += 1
				kernel32.CloseHandle(client)
			kernel32.DisconnectNamedPipe(server)

		kernel32.CloseHandle(server)
		self.assertEqual(successes, iterations)

	def test_pipe_resource_leak_checks(self):
		proc = kernel32.GetCurrentProcess()
		initial_count = wintypes.DWORD()
		kernel32.GetProcessHandleCount(proc, ctypes.byref(initial_count))

		pipe_name = r"\\.\pipe\whantv_leak_check_" + str(uuid.uuid4())[:8]
		for _ in range(50):
			server = kernel32.CreateNamedPipeW(
				pipe_name,
				PIPE_ACCESS_DUPLEX,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
				PIPE_UNLIMITED_INSTANCES,
				4096,
				4096,
				0,
				None
			)
			client = kernel32.CreateFileW(
				pipe_name,
				GENERIC_READ | GENERIC_WRITE,
				0,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None
			)
			if client != INVALID_HANDLE_VALUE:
				kernel32.CloseHandle(client)
			kernel32.CloseHandle(server)

		final_count = wintypes.DWORD()
		kernel32.GetProcessHandleCount(proc, ctypes.byref(final_count))
		delta = final_count.value - initial_count.value
		self.assertLessEqual(delta, 2)

	def test_socket_reset_handling(self):
		pipe_name = r"\\.\pipe\whantv_reset_" + str(uuid.uuid4())[:8]
		server = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		self.assertNotEqual(server, INVALID_HANDLE_VALUE)

		t1 = threading.Thread(target=lambda: kernel32.ConnectNamedPipe(server, None))
		t1.start()

		client1 = kernel32.CreateFileW(
			pipe_name,
			GENERIC_READ | GENERIC_WRITE,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		t1.join(timeout=0.5)

		msg = b"reset_test"
		bw = wintypes.DWORD()
		kernel32.WriteFile(client1, msg, len(msg), ctypes.byref(bw), None)
		kernel32.CloseHandle(client1)

		buf = ctypes.create_string_buffer(256)
		br = wintypes.DWORD()
		res = kernel32.ReadFile(server, buf, 256, ctypes.byref(br), None)
		err = kernel32.GetLastError()
		if not res:
			self.assertIn(err, [ERROR_BROKEN_PIPE, 0])

		kernel32.DisconnectNamedPipe(server)

		t2 = threading.Thread(target=lambda: kernel32.ConnectNamedPipe(server, None))
		t2.start()

		client2 = kernel32.CreateFileW(
			pipe_name,
			GENERIC_READ | GENERIC_WRITE,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		t2.join(timeout=0.5)

		self.assertNotEqual(client2, INVALID_HANDLE_VALUE)
		kernel32.CloseHandle(client2)
		kernel32.CloseHandle(server)

	def test_rapid_handle_exhaustion_recovery(self):
		pipe_base = r"\\.\pipe\whantv_exhaust_"
		created_handles = []
		for i in range(30):
			name = pipe_base + str(i) + "_" + str(uuid.uuid4())[:6]
			h = kernel32.CreateNamedPipeW(
				name,
				PIPE_ACCESS_DUPLEX,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
				PIPE_UNLIMITED_INSTANCES,
				1024,
				1024,
				0,
				None
			)
			self.assertNotEqual(h, INVALID_HANDLE_VALUE)
			created_handles.append(h)

		for h in created_handles:
			kernel32.CloseHandle(h)

		fresh_pipe = r"\\.\pipe\whantv_exhaust_fresh_" + str(uuid.uuid4())[:8]
		fresh_server = kernel32.CreateNamedPipeW(
			fresh_pipe,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		self.assertNotEqual(fresh_server, INVALID_HANDLE_VALUE)
		kernel32.CloseHandle(fresh_server)

	def test_reconnect_backoff_stability(self):
		pipe_name = r"\\.\pipe\whantv_backoff_" + str(uuid.uuid4())[:8]
		connected = [False]

		def server_delayed():
			time.sleep(0.1)
			server = kernel32.CreateNamedPipeW(
				pipe_name,
				PIPE_ACCESS_DUPLEX,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
				PIPE_UNLIMITED_INSTANCES,
				4096,
				4096,
				0,
				None
			)
			kernel32.ConnectNamedPipe(server, None)
			time.sleep(0.1)
			kernel32.CloseHandle(server)

		st = threading.Thread(target=server_delayed)
		st.start()

		attempts = 0
		max_attempts = 20
		backoff_ms = 0.02
		while attempts < max_attempts:
			attempts += 1
			client = kernel32.CreateFileW(
				pipe_name,
				GENERIC_READ | GENERIC_WRITE,
				0,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None
			)
			if client != INVALID_HANDLE_VALUE:
				connected[0] = True
				kernel32.CloseHandle(client)
				break
			time.sleep(backoff_ms)

		st.join(timeout=2.0)
		self.assertTrue(connected[0])

if __name__ == "__main__":
	unittest.main()
