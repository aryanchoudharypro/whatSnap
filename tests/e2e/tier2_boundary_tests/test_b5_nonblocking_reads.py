import unittest
import ctypes
from ctypes import wintypes
import os
import sys
import time
import uuid
import threading
import hashlib

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

kernel32.PeekNamedPipe.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.POINTER(wintypes.DWORD), ctypes.POINTER(wintypes.DWORD)]
kernel32.PeekNamedPipe.restype = wintypes.BOOL

kernel32.GetLastError.argtypes = []
kernel32.GetLastError.restype = wintypes.DWORD

class TestNonblockingReads(unittest.TestCase):

	def test_partial_frame_reads(self):
		pipe_name = r"\\.\pipe\whantv_partial_" + str(uuid.uuid4())[:8]
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

		full_msg = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789" * 3
		chunk_size = 10

		def writer():
			bw = wintypes.DWORD()
			for i in range(0, len(full_msg), chunk_size):
				chunk = full_msg[i:i + chunk_size]
				kernel32.WriteFile(client, chunk, len(chunk), ctypes.byref(bw), None)
				time.sleep(0.01)

		wt = threading.Thread(target=writer)
		wt.start()

		received = bytearray()
		buf = ctypes.create_string_buffer(chunk_size)
		br = wintypes.DWORD()
		while len(received) < len(full_msg):
			if kernel32.ReadFile(server, buf, chunk_size, ctypes.byref(br), None):
				received.extend(buf.raw[:br.value])

		wt.join(timeout=2.0)
		kernel32.CloseHandle(client)
		kernel32.CloseHandle(server)
		self.assertEqual(bytes(received), full_msg)

	def test_pipe_buffer_overflow_handling(self):
		pipe_name = r"\\.\pipe\whantv_overflow_" + str(uuid.uuid4())[:8]
		buf_size = 4096
		server = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			buf_size,
			buf_size,
			0,
			None
		)
		self.assertNotEqual(server, INVALID_HANDLE_VALUE)

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

		total_size = 65536
		payload = os.urandom(total_size)
		received = bytearray()

		def writer():
			bw = wintypes.DWORD()
			chunk = 4096
			for i in range(0, total_size, chunk):
				kernel32.WriteFile(client, payload[i:i + chunk], chunk, ctypes.byref(bw), None)

		def reader():
			buf = ctypes.create_string_buffer(4096)
			br = wintypes.DWORD()
			while len(received) < total_size:
				if kernel32.ReadFile(server, buf, 4096, ctypes.byref(br), None):
					received.extend(buf.raw[:br.value])

		wt = threading.Thread(target=writer)
		rt = threading.Thread(target=reader)
		wt.start()
		rt.start()

		wt.join(timeout=3.0)
		rt.join(timeout=3.0)

		kernel32.CloseHandle(client)
		kernel32.CloseHandle(server)
		self.assertEqual(len(received), total_size)
		self.assertEqual(bytes(received), payload)

	def test_read_write_timeout(self):
		pipe_name = r"\\.\pipe\whantv_timeout_" + str(uuid.uuid4())[:8]
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

		avail = wintypes.DWORD()
		start_time = time.time()
		timeout_sec = 0.2
		timed_out = False

		while time.time() - start_time < timeout_sec:
			res = kernel32.PeekNamedPipe(server, None, 0, None, ctypes.byref(avail), None)
			if res and avail.value > 0:
				break
			time.sleep(0.02)
		else:
			timed_out = True

		kernel32.CloseHandle(client)
		kernel32.CloseHandle(server)
		self.assertTrue(timed_out)

	def test_large_payload_over_pipe(self):
		pipe_name = r"\\.\pipe\whantv_large_" + str(uuid.uuid4())[:8]
		server = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			65536,
			65536,
			0,
			None
		)
		self.assertNotEqual(server, INVALID_HANDLE_VALUE)

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

		payload_size = 1048576
		pattern = b"0123456789ABCDEF" * (payload_size // 16)
		src_hash = hashlib.sha256(pattern).hexdigest()

		received_hash = [None]

		def writer():
			bw = wintypes.DWORD()
			chunk_size = 65536
			for i in range(0, payload_size, chunk_size):
				kernel32.WriteFile(client, pattern[i:i + chunk_size], chunk_size, ctypes.byref(bw), None)

		def reader():
			buf = ctypes.create_string_buffer(65536)
			br = wintypes.DWORD()
			h = hashlib.sha256()
			read_bytes = 0
			while read_bytes < payload_size:
				if kernel32.ReadFile(server, buf, 65536, ctypes.byref(br), None):
					h.update(buf.raw[:br.value])
					read_bytes += br.value
			received_hash[0] = h.hexdigest()

		wt = threading.Thread(target=writer)
		rt = threading.Thread(target=reader)
		wt.start()
		rt.start()

		wt.join(timeout=5.0)
		rt.join(timeout=5.0)

		kernel32.CloseHandle(client)
		kernel32.CloseHandle(server)
		self.assertEqual(received_hash[0], src_hash)

	def test_zero_byte_payload_boundary(self):
		pipe_name = r"\\.\pipe\whantv_zero_" + str(uuid.uuid4())[:8]
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

		bw = wintypes.DWORD()
		res_write = kernel32.WriteFile(client, b"", 0, ctypes.byref(bw), None)
		self.assertTrue(res_write)
		self.assertEqual(bw.value, 0)

		avail = wintypes.DWORD()
		res_peek = kernel32.PeekNamedPipe(server, None, 0, None, ctypes.byref(avail), None)
		self.assertTrue(res_peek)
		self.assertEqual(avail.value, 0)

		kernel32.CloseHandle(client)
		kernel32.CloseHandle(server)

if __name__ == "__main__":
	unittest.main()
