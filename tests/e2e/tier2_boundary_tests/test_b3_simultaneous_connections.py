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

ERROR_PIPE_BUSY = 231

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

kernel32.GetLastError.argtypes = []
kernel32.GetLastError.restype = wintypes.DWORD

class TestSimultaneousConnections(unittest.TestCase):

	def test_multiple_concurrent_client_connections(self):
		pipe_name = r"\\.\pipe\whantv_concurrent_" + str(uuid.uuid4())[:8]
		num_clients = 5
		servers = []
		for _ in range(num_clients):
			s = kernel32.CreateNamedPipeW(
				pipe_name,
				PIPE_ACCESS_DUPLEX,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
				num_clients,
				4096,
				4096,
				0,
				None
			)
			self.assertNotEqual(s, INVALID_HANDLE_VALUE)
			servers.append(s)

		results = [False] * num_clients

		def echo_server(idx):
			s_handle = servers[idx]
			kernel32.ConnectNamedPipe(s_handle, None)
			buf = ctypes.create_string_buffer(256)
			bytes_read = wintypes.DWORD()
			if kernel32.ReadFile(s_handle, buf, 256, ctypes.byref(bytes_read), None):
				bytes_written = wintypes.DWORD()
				kernel32.WriteFile(s_handle, buf, bytes_read.value, ctypes.byref(bytes_written), None)

		server_threads = [threading.Thread(target=echo_server, args=(i,)) for i in range(num_clients)]
		for t in server_threads:
			t.start()

		def client_worker(idx):
			time.sleep(0.05)
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
				msg = f"ping_{idx}".encode('utf-8')
				bw = wintypes.DWORD()
				kernel32.WriteFile(client, msg, len(msg), ctypes.byref(bw), None)
				buf = ctypes.create_string_buffer(256)
				br = wintypes.DWORD()
				if kernel32.ReadFile(client, buf, 256, ctypes.byref(br), None):
					if buf.value == msg:
						results[idx] = True
				kernel32.CloseHandle(client)

		client_threads = [threading.Thread(target=client_worker, args=(i,)) for i in range(num_clients)]
		for t in client_threads:
			t.start()

		for t in client_threads:
			t.join(timeout=3.0)
		for t in server_threads:
			t.join(timeout=3.0)

		for s in servers:
			kernel32.CloseHandle(s)

		self.assertTrue(all(results))

	def test_pipe_instance_limit(self):
		pipe_name = r"\\.\pipe\whantv_limit_" + str(uuid.uuid4())[:8]
		max_inst = 2
		servers = []
		for _ in range(max_inst):
			s = kernel32.CreateNamedPipeW(
				pipe_name,
				PIPE_ACCESS_DUPLEX,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
				max_inst,
				4096,
				4096,
				0,
				None
			)
			self.assertNotEqual(s, INVALID_HANDLE_VALUE)
			servers.append(s)

		clients = []
		for _ in range(max_inst):
			c = kernel32.CreateFileW(
				pipe_name,
				GENERIC_READ | GENERIC_WRITE,
				0,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None
			)
			self.assertNotEqual(c, INVALID_HANDLE_VALUE)
			clients.append(c)

		excess_client = kernel32.CreateFileW(
			pipe_name,
			GENERIC_READ | GENERIC_WRITE,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		err = kernel32.GetLastError()
		self.assertEqual(excess_client, INVALID_HANDLE_VALUE)
		self.assertEqual(err, ERROR_PIPE_BUSY)

		for c in clients:
			kernel32.CloseHandle(c)
		for s in servers:
			kernel32.CloseHandle(s)

	def test_request_multiplexing(self):
		pipe_name = r"\\.\pipe\whantv_multi_" + str(uuid.uuid4())[:8]
		num_channels = 3
		servers = []
		for _ in range(num_channels):
			s = kernel32.CreateNamedPipeW(
				pipe_name,
				PIPE_ACCESS_DUPLEX,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
				num_channels,
				4096,
				4096,
				0,
				None
			)
			servers.append(s)

		running = [True]
		def echo_loop(s_handle):
			kernel32.ConnectNamedPipe(s_handle, None)
			buf = ctypes.create_string_buffer(256)
			br = wintypes.DWORD()
			bw = wintypes.DWORD()
			while running[0]:
				if kernel32.ReadFile(s_handle, buf, 256, ctypes.byref(br), None):
					if br.value > 0:
						kernel32.WriteFile(s_handle, buf, br.value, ctypes.byref(bw), None)
					else:
						break
				else:
					break

		st_list = [threading.Thread(target=echo_loop, args=(s,)) for s in servers]
		for st in st_list:
			st.start()

		clients = []
		for _ in range(num_channels):
			c = kernel32.CreateFileW(
				pipe_name,
				GENERIC_READ | GENERIC_WRITE,
				0,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None
			)
			self.assertNotEqual(c, INVALID_HANDLE_VALUE)
			clients.append(c)

		mux_results = [True] * num_channels
		def client_loop(c_handle, chan_id):
			bw = wintypes.DWORD()
			br = wintypes.DWORD()
			buf = ctypes.create_string_buffer(256)
			for iteration in range(5):
				payload = f"chan_{chan_id}_seq_{iteration}".encode('utf-8')
				kernel32.WriteFile(c_handle, payload, len(payload), ctypes.byref(bw), None)
				if kernel32.ReadFile(c_handle, buf, 256, ctypes.byref(br), None):
					if buf.value != payload:
						mux_results[chan_id] = False
				else:
					mux_results[chan_id] = False

		ct_list = [threading.Thread(target=client_loop, args=(clients[i], i)) for i in range(num_channels)]
		for ct in ct_list:
			ct.start()

		for ct in ct_list:
			ct.join(timeout=3.0)

		running[0] = False
		for c in clients:
			kernel32.CloseHandle(c)
		for s in servers:
			kernel32.DisconnectNamedPipe(s)
			kernel32.CloseHandle(s)

		self.assertTrue(all(mux_results))

	def test_client_disconnect_isolation(self):
		pipe_name = r"\\.\pipe\whantv_disconn_iso_" + str(uuid.uuid4())[:8]
		num_inst = 3
		servers = []
		for _ in range(num_inst):
			s = kernel32.CreateNamedPipeW(
				pipe_name,
				PIPE_ACCESS_DUPLEX,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
				num_inst,
				4096,
				4096,
				0,
				None
			)
			servers.append(s)

		clients = []
		for _ in range(num_inst):
			c = kernel32.CreateFileW(
				pipe_name,
				GENERIC_READ | GENERIC_WRITE,
				0,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None
			)
			clients.append(c)

		kernel32.CloseHandle(clients[1])

		payload = b"isolation_test"
		bw = wintypes.DWORD()
		res0 = kernel32.WriteFile(clients[0], payload, len(payload), ctypes.byref(bw), None)
		res2 = kernel32.WriteFile(clients[2], payload, len(payload), ctypes.byref(bw), None)

		self.assertTrue(res0)
		self.assertTrue(res2)

		kernel32.CloseHandle(clients[0])
		kernel32.CloseHandle(clients[2])
		for s in servers:
			kernel32.CloseHandle(s)

	def test_concurrent_read_write_stress(self):
		pipe_name = r"\\.\pipe\whantv_stress_" + str(uuid.uuid4())[:8]
		workers = 4
		iters = 10
		servers = []
		for _ in range(workers):
			s = kernel32.CreateNamedPipeW(
				pipe_name,
				PIPE_ACCESS_DUPLEX,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
				workers,
				4096,
				4096,
				0,
				None
			)
			servers.append(s)

		running = [True]
		def server_loop(s_handle):
			kernel32.ConnectNamedPipe(s_handle, None)
			buf = ctypes.create_string_buffer(256)
			br = wintypes.DWORD()
			bw = wintypes.DWORD()
			while running[0]:
				if kernel32.ReadFile(s_handle, buf, 256, ctypes.byref(br), None):
					if br.value > 0:
						kernel32.WriteFile(s_handle, buf, br.value, ctypes.byref(bw), None)
					else:
						break
				else:
					break

		s_threads = [threading.Thread(target=server_loop, args=(s,)) for s in servers]
		for st in s_threads:
			st.start()

		successes = [0] * workers
		def worker_job(idx):
			time.sleep(0.02)
			c = kernel32.CreateFileW(
				pipe_name,
				GENERIC_READ | GENERIC_WRITE,
				0,
				None,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				None
			)
			if c == INVALID_HANDLE_VALUE:
				return
			bw = wintypes.DWORD()
			br = wintypes.DWORD()
			buf = ctypes.create_string_buffer(256)
			for i in range(iters):
				data = f"worker_{idx}_item_{i}".encode('utf-8')
				if kernel32.WriteFile(c, data, len(data), ctypes.byref(bw), None):
					if kernel32.ReadFile(c, buf, 256, ctypes.byref(br), None):
						if buf.value == data:
							successes[idx] += 1
			kernel32.CloseHandle(c)

		w_threads = [threading.Thread(target=worker_job, args=(i,)) for i in range(workers)]
		for wt in w_threads:
			wt.start()
		for wt in w_threads:
			wt.join(timeout=3.0)

		running[0] = False
		for s in servers:
			kernel32.DisconnectNamedPipe(s)
			kernel32.CloseHandle(s)

		self.assertEqual(sum(successes), workers * iters)

if __name__ == "__main__":
	unittest.main()
