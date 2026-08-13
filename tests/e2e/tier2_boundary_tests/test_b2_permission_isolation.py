import unittest
import ctypes
from ctypes import wintypes
import os
import sys
import uuid

kernel32 = ctypes.windll.kernel32

INVALID_HANDLE_VALUE = wintypes.HANDLE(-1).value
GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
WRITE_DAC = 0x04000000
OPEN_EXISTING = 3
FILE_ATTRIBUTE_NORMAL = 0x80

PIPE_ACCESS_OUTBOUND = 0x00000001
PIPE_ACCESS_INBOUND = 0x00000002
PIPE_ACCESS_DUPLEX = 0x00000003
PIPE_TYPE_BYTE = 0x00000000
PIPE_READMODE_BYTE = 0x00000000
PIPE_WAIT = 0x00000000
PIPE_UNLIMITED_INSTANCES = 255

ERROR_ACCESS_DENIED = 5
HANDLE_FLAG_INHERIT = 0x00000001

class SECURITY_ATTRIBUTES(ctypes.Structure):
	_fields_ = [
		("nLength", wintypes.DWORD),
		("lpSecurityDescriptor", wintypes.LPVOID),
		("bInheritHandle", wintypes.BOOL)
	]

kernel32.CreateNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(SECURITY_ATTRIBUTES)]
kernel32.CreateNamedPipeW.restype = wintypes.HANDLE

kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
kernel32.CreateFileW.restype = wintypes.HANDLE

kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL

kernel32.GetHandleInformation.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
kernel32.GetHandleInformation.restype = wintypes.BOOL

kernel32.GetLastError.argtypes = []
kernel32.GetLastError.restype = wintypes.DWORD

class TestPermissionIsolation(unittest.TestCase):

	def test_pipe_access_security(self):
		pipe_name = r"\\.\pipe\whantv_inbound_only_" + str(uuid.uuid4())[:8]
		server = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_INBOUND,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		self.assertNotEqual(server, INVALID_HANDLE_VALUE)

		client_read = kernel32.CreateFileW(
			pipe_name,
			GENERIC_READ,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		self.assertNotEqual(client_read, INVALID_HANDLE_VALUE)
		kernel32.CloseHandle(client_read)
		kernel32.CloseHandle(server)

		server2 = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_INBOUND,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			None
		)
		client_write = kernel32.CreateFileW(
			pipe_name,
			GENERIC_WRITE,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		err = kernel32.GetLastError()
		self.assertEqual(client_write, INVALID_HANDLE_VALUE)
		self.assertEqual(err, ERROR_ACCESS_DENIED)
		kernel32.CloseHandle(server2)

	def test_user_session_isolation(self):
		remote_pipe = r"\\invalid_host_session\pipe\whantv_session_" + str(uuid.uuid4())[:8]
		client = kernel32.CreateFileW(
			remote_pipe,
			GENERIC_READ | GENERIC_WRITE,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		err = kernel32.GetLastError()
		self.assertEqual(client, INVALID_HANDLE_VALUE)
		self.assertNotEqual(err, 0)

	def test_acl_checks(self):
		pipe_name = r"\\.\pipe\whantv_acl_check_" + str(uuid.uuid4())[:8]
		sa = SECURITY_ATTRIBUTES()
		sa.nLength = ctypes.sizeof(SECURITY_ATTRIBUTES)
		sa.lpSecurityDescriptor = None
		sa.bInheritHandle = False

		server = kernel32.CreateNamedPipeW(
			pipe_name,
			PIPE_ACCESS_DUPLEX,
			PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
			PIPE_UNLIMITED_INSTANCES,
			4096,
			4096,
			0,
			ctypes.byref(sa)
		)
		self.assertNotEqual(server, INVALID_HANDLE_VALUE)

		flags = wintypes.DWORD()
		res = kernel32.GetHandleInformation(server, ctypes.byref(flags))
		self.assertTrue(res)
		self.assertEqual(flags.value & HANDLE_FLAG_INHERIT, 0)
		kernel32.CloseHandle(server)

	def test_local_system_pipe_access(self):
		pipe_name = r"\\.\pipe\whantv_local_sys_" + str(uuid.uuid4())[:8]
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

		client = kernel32.CreateFileW(
			pipe_name,
			GENERIC_READ | GENERIC_WRITE,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		self.assertNotEqual(client, INVALID_HANDLE_VALUE)
		kernel32.CloseHandle(client)
		kernel32.CloseHandle(server)

	def test_unauthorized_access_prevention(self):
		pipe_name = r"\\.\pipe\whantv_unauth_" + str(uuid.uuid4())[:8]
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

		client = kernel32.CreateFileW(
			pipe_name,
			WRITE_DAC,
			0,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None
		)
		err = kernel32.GetLastError()
		self.assertEqual(client, INVALID_HANDLE_VALUE)
		self.assertEqual(err, ERROR_ACCESS_DENIED)
		kernel32.CloseHandle(server)

if __name__ == "__main__":
	unittest.main()
