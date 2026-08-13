import os
import sys
import time
import subprocess
import tempfile
import unittest
import psutil
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

class TestSeq2AbruptParentCrash(unittest.TestCase):
	def setUp(self):
		self.temp_dir = tempfile.mkdtemp(prefix="whantv_t3_seq2_")
		self.db_path = os.path.join(self.temp_dir, "seq2.db")
		self.pipe_name = r"\\.\pipe\whantv-ipc-seq2-test"
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

	def test_abrupt_parent_crash_job_object_cleanup(self):
		script_code = f"""
import os
import sys
import time
import subprocess
import ctypes
import ctypes.wintypes

JOBOBJECT_EXTENDED_LIMIT_INFORMATION = 9
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000

class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
	_fields_ = [
		("PerProcessUserTimeLimit", ctypes.wintypes.LARGE_INTEGER),
		("PerJobUserTimeLimit", ctypes.wintypes.LARGE_INTEGER),
		("LimitFlags", ctypes.wintypes.DWORD),
		("MinimumWorkingSetSize", ctypes.c_size_t),
		("MaximumWorkingSetSize", ctypes.c_size_t),
		("ActiveProcessLimit", ctypes.wintypes.DWORD),
		("Affinity", ctypes.POINTER(ctypes.wintypes.ULONG)),
		("PriorityClass", ctypes.wintypes.DWORD),
		("SchedulingClass", ctypes.wintypes.DWORD),
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

class JOBOBJECT_EXTENDED_LIMIT_INFORMATION_STRUCT(ctypes.Structure):
	_fields_ = [
		("BasicLimitInformation", JOBOBJECT_BASIC_LIMIT_INFORMATION),
		("IoInfo", IO_COUNTERS),
		("ProcessMemoryLimit", ctypes.c_size_t),
		("JobMemoryLimit", ctypes.c_size_t),
		("PeakProcessMemoryUsed", ctypes.c_size_t),
		("PeakJobMemoryUsed", ctypes.c_size_t),
	]

job_handle = ctypes.windll.kernel32.CreateJobObjectW(None, None)
info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION_STRUCT()
info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
ctypes.windll.kernel32.SetInformationJobObject(
	job_handle,
	JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
	ctypes.byref(info),
	ctypes.sizeof(info)
)

cmd = [{repr(self.backend_bin)}, "-db", {repr(self.db_path)}, "-pipe", {repr(self.pipe_name)}]
proc = subprocess.Popen(cmd)
ctypes.windll.kernel32.AssignProcessToJobObject(job_handle, int(proc._handle))
print("CHILD_PID:" + str(proc.pid), flush=True)

while True:
	time.sleep(1)
"""

		wrapper_path = os.path.join(self.temp_dir, "job_wrapper.py")
		with open(wrapper_path, "w", encoding="utf-8") as f:
			f.write(script_code)

		parent_proc = subprocess.Popen(
			[sys.executable, wrapper_path],
			stdout=subprocess.PIPE,
			stderr=subprocess.PIPE,
			text=True
		)

		child_pid = None
		for _ in range(50):
			line = parent_proc.stdout.readline()
			if "CHILD_PID:" in line:
				child_pid = int(line.strip().split("CHILD_PID:")[1])
				break
			time.sleep(0.1)

		if parent_proc.stdout:
			parent_proc.stdout.close()
		if parent_proc.stderr:
			parent_proc.stderr.close()

		self.assertIsNotNone(child_pid, "Failed to get child process PID from job wrapper")

		connected = False
		for _ in range(50):
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
				connected = True
				break
			except Exception:
				time.sleep(0.1)

		self.assertTrue(connected, "Backend process was not accessible over named pipe")

		subprocess.run(["taskkill", "/F", "/PID", str(parent_proc.pid)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
		parent_proc.wait()

		time.sleep(1.0)
		child_alive = psutil.pid_exists(child_pid)
		if child_alive:
			try:
				p = psutil.Process(child_pid)
				if p.status() == psutil.STATUS_ZOMBIE:
					child_alive = False
			except psutil.NoSuchProcess:
				child_alive = False

		self.assertFalse(child_alive, "Win32 Job Object failed to auto-kill backend process after parent crash")

		stale_pipe_closed = False
		try:
			h_stale = win32file.CreateFile(
				self.pipe_name,
				win32file.GENERIC_READ | win32file.GENERIC_WRITE,
				0,
				None,
				win32file.OPEN_EXISTING,
				0,
				None
			)
			win32file.CloseHandle(h_stale)
		except Exception:
			stale_pipe_closed = True

		self.assertTrue(stale_pipe_closed, "Stale pipe remained active after backend process auto-kill")

		recovery_db = os.path.join(self.temp_dir, "recovery.db")
		rec_cmd = [self.backend_bin, "-db", recovery_db, "-pipe", self.pipe_name]
		rec_proc = subprocess.Popen(rec_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

		rec_connected = False
		for _ in range(50):
			try:
				h_rec = win32file.CreateFile(
					self.pipe_name,
					win32file.GENERIC_READ | win32file.GENERIC_WRITE,
					0,
					None,
					win32file.OPEN_EXISTING,
					0,
					None
				)
				shutdown_req = b"POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n"
				win32file.WriteFile(h_rec, shutdown_req)
				err, resp = win32file.ReadFile(h_rec, 4096)
				win32file.CloseHandle(h_rec)
				if err == 0 and b"HTTP/1.1 200 OK" in resp:
					rec_connected = True
					break
			except Exception:
				time.sleep(0.1)

		rec_proc.wait(timeout=5)
		self.assertTrue(rec_connected, "Failed clean pipe recovery and restart after abrupt parent crash")

if __name__ == "__main__":
	unittest.main()
