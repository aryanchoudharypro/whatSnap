import unittest
import sys
import os
import time
import argparse

class FormattedTestResult(unittest.TextTestResult):
	def __init__(self, stream, descriptions, verbosity):
		super().__init__(stream, descriptions, verbosity)
		self.passed_count = 0

	def addSuccess(self, test):
		super().addSuccess(test)
		self.passed_count += 1

def run_e2e_tests(tier_filter="all", verbose=1):
	base_dir = os.path.dirname(os.path.abspath(__file__))
	tier_dirs = {
		"tier1": os.path.join(base_dir, "tier1_feature_tests"),
		"tier2": os.path.join(base_dir, "tier2_boundary_tests"),
		"tier3": os.path.join(base_dir, "tier3_cross_feature_tests"),
		"tier4": os.path.join(base_dir, "tier4_scenario_tests"),
	}

	selected_dirs = []
	normalized_filter = str(tier_filter).lower().strip()

	if normalized_filter in ("all", "0"):
		selected_dirs = list(tier_dirs.values())
	elif normalized_filter in ("tier1", "1"):
		selected_dirs = [tier_dirs["tier1"]]
	elif normalized_filter in ("tier2", "2"):
		selected_dirs = [tier_dirs["tier2"]]
	elif normalized_filter in ("tier3", "3"):
		selected_dirs = [tier_dirs["tier3"]]
	elif normalized_filter in ("tier4", "4"):
		selected_dirs = [tier_dirs["tier4"]]
	else:
		selected_dirs = list(tier_dirs.values())

	loader = unittest.TestLoader()
	master_suite = unittest.TestSuite()

	for target_dir in selected_dirs:
		if os.path.exists(target_dir):
			suite = loader.discover(start_dir=target_dir, pattern="test_*.py")
			master_suite.addTest(suite)

	stream = sys.stdout
	runner = unittest.TextTestRunner(stream=stream, resultclass=FormattedTestResult, verbosity=verbose)

	print("=" * 70)
	print("       whatSnap E2E Test Suite Orchestrator & Execution")
	print("=" * 70)
	print(f"Target Tier: {tier_filter}")

	start_time = time.time()
	result = runner.run(master_suite)
	elapsed_time = time.time() - start_time

	total_tests = result.testsRun
	failed_tests = len(result.failures)
	errored_tests = len(result.errors)
	skipped_tests = len(result.skipped)
	passed_tests = getattr(result, "passed_count", total_tests - failed_tests - errored_tests - skipped_tests)

	is_success = result.wasSuccessful()

	print("\n" + "=" * 70)
	print("                      TEST EXECUTION SUMMARY")
	print("=" * 70)
	print(f"Total Test Cases Executed : {total_tests}")
	print(f"Passed                    : {passed_tests}")
	print(f"Failed                    : {failed_tests}")
	print(f"Errors                    : {errored_tests}")
	print(f"Skipped                   : {skipped_tests}")
	print(f"Elapsed Time              : {elapsed_time:.3f} seconds")
	print(f"Overall Result            : {'PASSED' if is_success else 'FAILED'}")
	print("=" * 70)

	return 0 if is_success else 1

if __name__ == "__main__":
	parser = argparse.ArgumentParser(description="whatSnap E2E Central Test Runner")
	parser.add_argument("--tier", type=str, default="all", help="Test tier to run (all, tier1, tier2, tier3, tier4)")
	parser.add_argument("--verbose", type=int, default=1, help="Verbosity level (0, 1, 2)")
	args = parser.parse_args()

	exit_code = run_e2e_tests(tier_filter=args.tier, verbose=args.verbose)
	sys.exit(exit_code)
