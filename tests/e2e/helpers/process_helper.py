import os
import sys
import time
import subprocess
import ctypes
from ctypes import wintypes

kernel32 = ctypes.windll.kernel32

JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000
JobObjectExtendedLimitInformation = 9
PROCESS_ALL_ACCESS = 0x001F0FFF
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_TERMINATE = 0x0001
INVALID_HANDLE_VALUE = wintypes.HANDLE(-1).value

class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
	_fields_ = [
		("PerProcessUserTimeLimit", wintypes.LARGE_INTEGER),
		("PerJobUserTimeLimit", wintypes.LARGE_INTEGER),
		("LimitFlags", wintypes.DWORD),
		("MinimumWorkingSetSize", ctypes.c_size_t),
		("MaximumWorkingSetSize", ctypes.c_size_t),
		("ActiveProcessLimit", wintypes.DWORD),
		("Affinity", ctypes.c_size_t),
		("PriorityClass", wintypes.DWORD),
		("SchedulingClass", wintypes.DWORD),
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

kernel32.CreateJobObjectW.argtypes = [wintypes.LPVOID, wintypes.LPCWSTR]
kernel32.CreateJobObjectW.restype = wintypes.HANDLE

kernel32.SetInformationJobObject.argtypes = [
	wintypes.HANDLE,
	ctypes.c_int,
	wintypes.LPVOID,
	wintypes.DWORD
]
kernel32.SetInformationJobObject.restype = wintypes.BOOL

kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
kernel32.AssignProcessToJobObject.restype = wintypes.BOOL

kernel32.IsProcessInJob.argtypes = [wintypes.HANDLE, wintypes.HANDLE, ctypes.POINTER(wintypes.BOOL)]
kernel32.IsProcessInJob.restype = wintypes.BOOL

kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel32.OpenProcess.restype = wintypes.HANDLE

kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
kernel32.TerminateProcess.restype = wintypes.BOOL

kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL

def launch_process(cmd_args, cwd=None, env=None, creationflags=0x08000000):
	if isinstance(cmd_args, str):
		cmd_args = cmd_args.split()
	proc = subprocess.Popen(
		cmd_args,
		cwd=cwd,
		env=env,
		creationflags=creationflags,
		stdout=subprocess.PIPE,
		stderr=subprocess.PIPE
	)
	return proc

def is_process_running(pid):
	if pid <= 0:
		return False
	handle = kernel32.OpenProcess(PROCESS_QUERY_INFORMATION, False, pid)
	if handle:
		exit_code = wintypes.DWORD()
		kernel32.GetExitCodeProcess(handle, ctypes.byref(exit_code))
		kernel32.CloseHandle(handle)
		return exit_code.value == 259
	return False

def terminate_process_by_pid(pid):
	handle = kernel32.OpenProcess(PROCESS_TERMINATE, False, pid)
	if handle:
		res = kernel32.TerminateProcess(handle, 1)
		kernel32.CloseHandle(handle)
		return res
	return False

def create_kill_on_close_job_object():
	job_handle = kernel32.CreateJobObjectW(None, None)
	if not job_handle or job_handle == INVALID_HANDLE_VALUE:
		return None
	info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
	info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	res = kernel32.SetInformationJobObject(
		job_handle,
		JobObjectExtendedLimitInformation,
		ctypes.byref(info),
		ctypes.sizeof(info)
	)
	if not res:
		kernel32.CloseHandle(job_handle)
		return None
	return job_handle

def assign_process_to_job(job_handle, process_handle_or_pid):
	if isinstance(process_handle_or_pid, int):
		proc_handle = kernel32.OpenProcess(PROCESS_ALL_ACCESS, False, process_handle_or_pid)
		if not proc_handle:
			return False
		res = kernel32.AssignProcessToJobObject(job_handle, proc_handle)
		kernel32.CloseHandle(proc_handle)
		return bool(res)
	else:
		return bool(kernel32.AssignProcessToJobObject(job_handle, process_handle_or_pid))

def is_process_in_job(process_pid, job_handle=None):
	proc_handle = kernel32.OpenProcess(PROCESS_QUERY_INFORMATION, False, process_pid)
	if not proc_handle:
		return False
	result = wintypes.BOOL(False)
	res = kernel32.IsProcessInJob(proc_handle, job_handle, ctypes.byref(result))
	kernel32.CloseHandle(proc_handle)
	return bool(res and result.value)

def get_temp_backend_path():
	temp_dir = os.environ.get("TEMP", os.environ.get("TMP", "C:\\Temp"))
	return os.path.join(temp_dir, "whantv", "backend.exe")

def check_temp_binary_extracted(path=None):
	if path is None:
		path = get_temp_backend_path()
	if not os.path.exists(path):
		return False
	if os.path.getsize(path) == 0:
		return False
	if not os.access(path, os.R_OK):
		return False
	return True

def get_child_pids(parent_pid):
	children = []
	try:
		output = subprocess.check_output(
			f"wmic process where (ParentProcessId={parent_pid}) get ProcessId",
			shell=True,
			text=True
		)
		lines = [line.strip() for line in output.splitlines() if line.strip()]
		for line in lines[1:]:
			if line.isdigit():
				children.append(int(line))
	except Exception:
		pass
	return children
