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

ERROR_FILE_NOT_FOUND = 2
ERROR_BROKEN_PIPE = 109

kernel32.CreateNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID]
kernel32.CreateNamedPipeW.restype = wintypes.HANDLE

kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
kernel32.CreateFileW.restype = wintypes.HANDLE

kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL

kernel32.ConnectNamedPipe.argtypes = [wintypes.HANDLE, wintypes.LPVOID]
kernel32.ConnectNamedPipe.restype = wintypes.BOOL

kernel32.DisconnectNamedPipe.argtypes = [wintypes.HANDLE]
kernel32.DisconnectNamedPipe.restype = wintypes.BOOL

kernel32.WaitNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD]
kernel32.WaitNamedPipeW.restype = wintypes.BOOL

kernel32.GetLastError.argtypes = []
kernel32.GetLastError.restype = wintypes.DWORD

class TestStalePipeHandling(unittest.TestCase):

	def test_connect_nonexistent_pipe(self):
		non_existent_pipe = r"\\.\pipe\whantv_nonexistent_" + str(uuid.uuid4())[:8]
		handle = kernel32.CreateFileW(
			non_existent_pipe,
			GENERIC_READ | GENERIC_WRITE,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		err = kernel32.GetLastError()
		self.assertEqual(handle, INVALID_HANDLE_VALUE)
		self.assertEqual(err, ERROR_FILE_NOT_FOUND)

	def test_stale_pipe_after_crash(self):
		pipe_name = r"\\.\pipe\whantv_stale_crash_" + str(uuid.uuid4())[:8]
		server_handle = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		self.assertNotEqual(server_handle, INVALID_HANDLE_VALUE)
		kernel32.CloseHandle(server_handle)

		client_handle = kernel32.CreateFileW(
			pipe_name,
			GENERIC_READ | GENERIC_WRITE,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		err = kernel32.GetLastError()
		self.assertEqual(client_handle, INVALID_HANDLE_VALUE)
		self.assertEqual(err, ERROR_FILE_NOT_FOUND)

		recreated_server = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		self.assertNotEqual(recreated_server, INVALID_HANDLE_VALUE)
		kernel32.CloseHandle(recreated_server)

	def test_pipe_recreation(self):
		pipe_name = r"\\.\pipe\whantv_recreate_" + str(uuid.uuid4())[:8]
		server1 = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		self.assertNotEqual(server1, INVALID_HANDLE_VALUE)
		kernel32.CloseHandle(server1)

		server2 = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		self.assertNotEqual(server2, INVALID_HANDLE_VALUE)

		client_connected = [False]
		def client_worker():
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
				client_connected[0] = True
				kernel32.CloseHandle(client)

		t = threading.Thread(target=client_worker)
		t.start()
		kernel32.ConnectNamedPipe(server2, None)
		t.join(timeout=2.0)
		kernel32.CloseHandle(server2)
		self.assertTrue(client_connected[0])

	def test_pipe_path_validation(self):
		invalid_paths = [
			"",
			"invalid_pipe_name",
			r"C:\temp\whantv_pipe",
			r"\\.\pipe\..\traversal_pipe",
			r"\\.\pipe\invalid*pipe?"
		]
		for path in invalid_paths:
			handle = kernel32.CreateFileW(
				path,
				GENERIC_READ | GENERIC_WRITE,
				0,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None
			)
			if handle != INVALID_HANDLE_VALUE:
				kernel32.CloseHandle(handle)
			self.assertEqual(handle, INVALID_HANDLE_VALUE)

	def test_connection_retries(self):
		pipe_name = r"\\.\pipe\whantv_retry_" + str(uuid.uuid4())[:8]
		server_ready = threading.Event()
		connected = [False]

		def server_thread():
			time.sleep(0.2)
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
			server_ready.set()
			kernel32.ConnectNamedPipe(server, None)
			time.sleep(0.1)
			kernel32.CloseHandle(server)

		t = threading.Thread(target=server_thread)
		t.start()

		start_time = time.time()
		max_wait = 2.0
		client_handle = INVALID_HANDLE_VALUE
		while time.time() - start_time < max_wait:
			if kernel32.WaitNamedPipeW(pipe_name, 50):
				client_handle = kernel32.CreateFileW(
					pipe_name,
					GENERIC_READ | GENERIC_WRITE,
					0,
					None,
					OPEN_EXISTING,
					FILE_ATTRIBUTE_NORMAL,
					None
				)
				if client_handle != INVALID_HANDLE_VALUE:
					connected[0] = True
					kernel32.CloseHandle(client_handle)
					break
			time.sleep(0.05)

		t.join(timeout=2.0)
		self.assertTrue(connected[0])

if __name__ == "__main__":
	unittest.main()
