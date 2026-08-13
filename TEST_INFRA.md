# E2E Test Infra: whaNTV IPC & Binary Embedding Migration

## Test Philosophy
- **Opaque-box & Requirement-driven**: Tests interact with `whantv.exe` and `backend.exe` strictly via external interfaces (Windows Named Pipe `\\.\pipe\whantv-ipc`, process lifecycle, Win32 Job Objects, and system state like `netstat`).
- **Methodology**: 4-Tier test architecture: Category-Partition Feature Coverage (Tier 1), Boundary Value & Edge Analysis (Tier 2), Cross-Feature Combinations (Tier 3), and Real-World Application Workloads (Tier 4).

## Feature Inventory & Test Coverage Mapping
| # | Feature | Requirement Source | Tier 1 | Tier 2 | Tier 3 | Tier 4 |
|---|---------|-------------------|:------:|:------:|:------:|:------:|
| 1 | Go Named Pipe Listener | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ | ✓ |
| 2 | Go Backend Executable Target | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ | ✓ |
| 3 | Rust Named Pipe Client | ORIGINAL_REQUEST §R2 | 5 | 5 | ✓ | ✓ |
| 4 | Single Binary Embedding | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ | ✓ |
| 5 | Temp Binary Extractor | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ | ✓ |
| 6 | Win32 Job Object Process Cleanup | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ | ✓ |
| 7 | Full E2E & Zero TCP Socket Verification | ORIGINAL_REQUEST Acceptance Criteria | 5 | 5 | ✓ | ✓ |

## Test Suite Architecture
- **Location**: `tests/e2e/`
- **Test Runner**: `python tests/e2e/runner.py` (or `powershell -ExecutionPolicy Bypass -File tests/e2e/run_tests.ps1`)
- **Structure**:
  - `tests/e2e/runner.py`: Central test harness and orchestrator for running all test tiers.
  - `tests/e2e/helpers/`: Win32 named pipe helpers (`pipe_client.py`), process monitors (`process_helper.py`), and netstat checkers (`netstat_helper.py`).
  - `tests/e2e/tier1_feature_tests/`: 35 standalone test cases verifying isolated features.
  - `tests/e2e/tier2_boundary_tests/`: 25 boundary & corner case test cases.
  - `tests/e2e/tier3_cross_feature_tests/`: 5 cross-feature integration test sequences.
  - `tests/e2e/tier4_scenario_tests/`: 5 real-world application workload scenarios.

## Coverage Thresholds
- **Tier 1 (Feature Coverage)**: 35 test cases (≥5 per feature)
- **Tier 2 (Boundary & Corner Cases)**: 25 test cases (≥5 per boundary area)
- **Tier 3 (Cross-Feature Interaction)**: 5 integration test sequences
- **Tier 4 (Real-World Scenarios)**: 5 application-level workload scenarios
- **Total Test Cases**: 70 E2E test cases

## Verification Commands
```powershell
python tests/e2e/runner.py --tier all
```
