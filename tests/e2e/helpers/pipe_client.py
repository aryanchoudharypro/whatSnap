import ctypes
from ctypes import wintypes
import time
import os
import struct

kernel32 = ctypes.windll.kernel32

GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
OPEN_EXISTING = 3
FILE_ATTRIBUTE_NORMAL = 0x80
INVALID_HANDLE_VALUE = wintypes.HANDLE(-1).value

kernel32.CreateFileW.argtypes = [
	wintypes.LPCWSTR,
	wintypes.DWORD,
	wintypes.DWORD,
	wintypes.LPVOID,
	wintypes.DWORD,
	wintypes.DWORD,
	wintypes.HANDLE
]
kernel32.CreateFileW.restype = wintypes.HANDLE

kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL

kernel32.ReadFile.argtypes = [
	wintypes.HANDLE,
	wintypes.LPVOID,
	wintypes.DWORD,
	ctypes.POINTER(wintypes.DWORD),
	wintypes.LPVOID
]
kernel32.ReadFile.restype = wintypes.BOOL

kernel32.WriteFile.argtypes = [
	wintypes.HANDLE,
	wintypes.LPCVOID,
	wintypes.DWORD,
	ctypes.POINTER(wintypes.DWORD),
	wintypes.LPVOID
]
kernel32.WriteFile.restype = wintypes.BOOL

kernel32.WaitNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD]
kernel32.WaitNamedPipeW.restype = wintypes.BOOL

kernel32.GetLastError.argtypes = []
kernel32.GetLastError.restype = wintypes.DWORD

class NamedPipeClient:
	def __init__(self, pipe_name=r"\\.\pipe\whantv-ipc", timeout_ms=5000):
		self.pipe_name = pipe_name
		self.timeout_ms = timeout_ms
		self.handle = INVALID_HANDLE_VALUE

	def connect(self):
		start_time = time.time()
		while (time.time() - start_time) * 1000 < self.timeout_ms:
			if kernel32.WaitNamedPipeW(self.pipe_name, 100):
				self.handle = kernel32.CreateFileW(
					self.pipe_name,
					GENERIC_READ | GENERIC_WRITE,
					0,
					None,
					OPEN_EXISTING,
					FILE_ATTRIBUTE_NORMAL,
					None
				)
				if self.handle != INVALID_HANDLE_VALUE:
					return True
			time.sleep(0.05)
		return False

	def close(self):
		if self.handle != INVALID_HANDLE_VALUE:
			kernel32.CloseHandle(self.handle)
			self.handle = INVALID_HANDLE_VALUE

	def is_connected(self):
		return self.handle != INVALID_HANDLE_VALUE

	def write(self, data: bytes) -> int:
		if self.handle == INVALID_HANDLE_VALUE:
			raise RuntimeError("Pipe not connected")
		written = wintypes.DWORD(0)
		buf = (ctypes.c_char * len(data)).from_buffer_copy(data)
		res = kernel32.WriteFile(self.handle, buf, len(data), ctypes.byref(written), None)
		if not res:
			raise RuntimeError(f"WriteFile failed with error {kernel32.GetLastError()}")
		return written.value

	def read(self, buffer_size=4096) -> bytes:
		if self.handle == INVALID_HANDLE_VALUE:
			raise RuntimeError("Pipe not connected")
		read_bytes = wintypes.DWORD(0)
		buf = ctypes.create_string_buffer(buffer_size)
		res = kernel32.ReadFile(self.handle, buf, buffer_size, ctypes.byref(read_bytes), None)
		if not res and read_bytes.value == 0:
			raise RuntimeError(f"ReadFile failed with error {kernel32.GetLastError()}")
		return buf.raw[:read_bytes.value]

	def send_http_request(self, method="GET", path="/", headers=None, body=b"") -> bytes:
		if headers is None:
			headers = {}
		headers_str = "".join([f"{k}: {v}\r\n" for k, v in headers.items()])
		req = f"{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {len(body)}\r\n{headers_str}\r\n".encode("utf-8") + body
		self.write(req)
		time.sleep(0.05)
		return self.read()

	def send_ws_handshake(self, path="/ws") -> bytes:
		headers = {
			"Upgrade": "websocket",
			"Connection": "Upgrade",
			"Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ==",
			"Sec-WebSocket-Version": "13"
		}
		return self.send_http_request(method="GET", path=path, headers=headers)

	def send_ws_frame(self, payload: bytes, opcode=0x1) -> int:
		first_byte = 0x80 | (opcode & 0x0F)
		length = len(payload)
		mask_key = b"\x12\x34\x56\x78"
		if length < 126:
			header = struct.pack("!BB", first_byte, 0x80 | length)
		elif length < 65536:
			header = struct.pack("!BBH", first_byte, 0x80 | 126, length)
		else:
			header = struct.pack("!BBQ", first_byte, 0x80 | 127, length)
		masked_payload = bytearray(length)
		for i in range(length):
			masked_payload[i] = payload[i] ^ mask_key[i % 4]
		frame = header + mask_key + bytes(masked_payload)
		return self.write(frame)

	def read_ws_frame(self) -> tuple:
		raw = self.read()
		if len(raw) < 2:
			return 0, b""
		first_byte = raw[0]
		opcode = first_byte & 0x0F
		second_byte = raw[1]
		masked = bool(second_byte & 0x80)
		length = second_byte & 0x7F
		offset = 2
		if length == 126:
			length = struct.unpack("!H", raw[2:4])[0]
			offset = 4
		elif length == 127:
			length = struct.unpack("!Q", raw[2:10])[0]
			offset = 10
		if masked:
			mask_key = raw[offset:offset+4]
			offset += 4
			payload_data = bytearray(raw[offset:offset+length])
			for i in range(len(payload_data)):
				payload_data[i] ^= mask_key[i % 4]
			return opcode, bytes(payload_data)
		else:
			return opcode, raw[offset:offset+length]

	def send_shutdown(self) -> bytes:
		return self.send_http_request(method="POST", path="/shutdown")
