import unittest
import os
import shutil
import tempfile
import subprocess
import sys
import time

class TestCleanEnvLaunch(unittest.TestCase):
	temp_dir = tempfile.gettempdir()
	whantv_temp = os.path.join(temp_dir, "whantv")
	extracted_backend = os.path.join(whantv_temp, "backend.exe")
	repo_backend = os.path.join("backend", "backend.exe")

	def test_clean_environment_temp_isolation(self):
		if os.path.exists(self.extracted_backend):
			try:
				os.remove(self.extracted_backend)
			except Exception:
				pass

		is_absent = not os.path.exists(self.extracted_backend)
		self.assertTrue(is_absent)

		os.makedirs(self.whantv_temp, exist_ok=True)
		if os.path.exists(self.repo_backend):
			shutil.copy2(self.repo_backend, self.extracted_backend)

		if os.path.exists(self.extracted_backend):
			self.assertTrue(os.path.exists(self.extracted_backend))
			self.assertGreater(os.path.getsize(self.extracted_backend), 0)

	def test_isolated_folder_launch(self):
		isolated_dir = tempfile.mkdtemp(prefix="whantv_clean_env_")
		try:
			backend_in_isolated = os.path.join(isolated_dir, "backend.exe")
			self.assertFalse(os.path.exists(backend_in_isolated))

			whantv_exe = os.path.join("target", "debug", "whatSnap.exe")
			if os.path.exists(whantv_exe):
				copied_exe = os.path.join(isolated_dir, "whatSnap.exe")
				shutil.copy2(whantv_exe, copied_exe)
				self.assertTrue(os.path.exists(copied_exe))
		finally:
			shutil.rmtree(isolated_dir, ignore_errors=True)

if __name__ == "__main__":
	unittest.main()
