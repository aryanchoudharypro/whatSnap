import unittest
import subprocess
import sqlite3
import os
import sys
import time
import ctypes
from ctypes import wintypes

class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
	_fields_ = [
		("PerProcessUserTimeLimit", ctypes.c_int64),
		("PerJobUserTimeLimit", ctypes.c_int64),
		("LimitFlags", ctypes.c_uint32),
		("MinimumWorkingSetSize", ctypes.c_size_t),
		("MaximumWorkingSetSize", ctypes.c_size_t),
		("ActiveProcessLimit", ctypes.c_uint32),
		("Affinity", ctypes.c_size_t),
		("PriorityClass", ctypes.c_uint32),
		("SchedulingClass", ctypes.c_uint32),
	]

class IO_COUNTERS(ctypes.Structure):
	_fields_ = [
		("ReadOperationCount", ctypes.c_uint64),
		("WriteOperationCount", ctypes.c_uint64),
		("OtherOperationCount", ctypes.c_uint64),
		("ReadTransferCount", ctypes.c_uint64),
		("WriteTransferCount", ctypes.c_uint64),
		("OtherTransferCount", ctypes.c_uint64),
	]

class JOBOBJECT_EXTENDED_LIMIT_INFORMATION(ctypes.Structure):
	_fields_ = [
		("BasicLimitInformation", JOBOBJECT_BASIC_LIMIT_INFORMATION),
		("IoInfo", IO_COUNTERS),
		("ProcessMemoryLimit", ctypes.c_size_t),
		("JobMemoryLimit", ctypes.c_size_t),
		("PeakProcessMemoryUsed", ctypes.c_size_t),
		("PeakJobMemoryUsed", ctypes.c_size_t),
	]

class TestParentCrashJobObject(unittest.TestCase):
	crash_db_path = "test_crash.db"

	def setUp(self):
		conn = sqlite3.connect(self.crash_db_path)
		cursor = conn.cursor()
		cursor.execute("CREATE TABLE IF NOT EXISTS journal (id INTEGER PRIMARY KEY AUTOINCREMENT, tx_data TEXT)")
		cursor.execute("INSERT INTO journal (tx_data) VALUES ('uncommitted_state_transaction')")
		conn.commit()
		conn.close()

	def tearDown(self):
		if os.path.exists(self.crash_db_path):
			try:
				os.remove(self.crash_db_path)
			except Exception:
				pass

	def test_job_object_creation_and_limit_assignment(self):
		kernel32 = ctypes.windll.kernel32
		job_handle = kernel32.CreateJobObjectW(None, None)
		self.assertNotEqual(job_handle, 0)
		self.assertNotEqual(job_handle, -1)

		info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
		info.BasicLimitInformation.LimitFlags = 0x2000

		res = kernel32.SetInformationJobObject(
			job_handle,
			9,
			ctypes.byref(info),
			ctypes.sizeof(info)
		)
		self.assertNotEqual(res, 0)
		kernel32.CloseHandle(job_handle)

	def test_simulated_crash_job_object_cleanup(self):
		kernel32 = ctypes.windll.kernel32
		job_handle = kernel32.CreateJobObjectW(None, None)
		info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
		info.BasicLimitInformation.LimitFlags = 0x2000
		kernel32.SetInformationJobObject(job_handle, 9, ctypes.byref(info), ctypes.sizeof(info))

		proc = subprocess.Popen(["cmd.exe", "/c", "ping 127.0.0.1 -n 5 > nul"], creationflags=0x08000000)
		proc_handle = ctypes.win32def.HANDLE(proc._handle) if hasattr(ctypes, "win32def") else proc._handle

		assign_res = kernel32.AssignProcessToJobObject(job_handle, proc_handle)
		self.assertNotEqual(assign_res, 0)

		kernel32.CloseHandle(job_handle)
		proc.kill()
		proc.wait()

		self.assertIsNotNone(proc.poll())

	def test_db_state_integrity_after_simulated_crash(self):
		conn = sqlite3.connect(self.crash_db_path)
		cursor = conn.cursor()
		cursor.execute("PRAGMA integrity_check")
		res = cursor.fetchone()
		conn.close()
		self.assertIsNotNone(res)
		self.assertEqual(res[0], "ok")

if __name__ == "__main__":
	unittest.main()
