#!/usr/bin/env python3
"""Synthetic privacy and progress-semantics tests for history diagnostics."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("history_diagnostics", Path(__file__).with_name("export-history-diagnostics.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

LINE = ("2026-10-19T21:00:00Z INFO zapfast [HISTORY] version=1 account=4 "
        "flow=initial stage=received sync_type=2 percent=100 chats=0 entries=0 elapsed_ms=0")


class HistoryExport(unittest.TestCase):
    def test_cli_filters_time_and_account_without_copying_other_log_data(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            source = root / "synthetic.log"
            output = root / "counters.json"
            source.write_text("PRIVATE SENTINEL\n" + LINE + "\n" +
                              LINE.replace("account=4", "account=2") + "\n" +
                              LINE.replace("21:00:00Z", "20:00:00Z") + "\n", encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(Path(module.__file__)), str(source),
                 "--since", "2026-10-19T20:30:00Z", "--account", "4", "--output", str(output)],
                capture_output=True, text=True,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            report = output.read_text(encoding="utf-8")
            self.assertNotIn("PRIVATE SENTINEL", report)
            self.assertEqual(len(json.loads(report)["records"]), 1)

    def test_valid_record_only_contains_fixed_labels_and_numbers(self):
        record = module.extract(LINE)
        self.assertEqual(record["flow"], "initial")
        self.assertEqual(record["percent"], 100)
        self.assertEqual(record["account"], 4)

    def test_rejects_private_extra_fields_and_malformed_records(self):
        for suffix in (" message=private", " phone=5511000000000", " key=secret", " qr=secret",
                       " account=5", " completion=true", " trailing private text"):
            with self.subTest(suffix=suffix):
                self.assertIsNone(module.extract(LINE + suffix))
        for before, after in (("version=1", "version=2"), ("percent=100", "percent=101"),
                              ("account=4", "account=-4"), ("flow=initial", "flow=private"),
                              ("2026-10-19", "2026-99-99"), ("entries=0", "entries=-1")):
            with self.subTest(after=after):
                self.assertIsNone(module.extract(LINE.replace(before, after)))

    def test_receipt_and_reported_100_percent_are_not_storage_or_completion(self):
        summary = module.summarize([module.extract(LINE)])[0]
        self.assertEqual(summary["events"], {"received": 1})
        self.assertEqual(summary["last_event_times"], {"received": "2026-10-19T21:00:00Z"})
        self.assertEqual(summary["stored_entries_including_repeats"], 0)
        self.assertNotIn("complete", summary)
        failed = module.extract(LINE.replace("stage=received", "stage=store_failed").replace("entries=0", "entries=42"))
        failed_summary = module.summarize([failed])[0]
        self.assertEqual(failed_summary["stored_entries_including_repeats"], 0)
        self.assertEqual(failed_summary["events"], {"store_failed": 1})

    def test_initial_requested_and_replayed_work_stay_separate(self):
        records = [module.extract(LINE.replace("stage=received", "stage=stored").replace("entries=0", "entries=42"))]
        for flow in ("on_demand", "replay"):
            records.append(module.extract(LINE.replace("flow=initial", f"flow={flow}").replace("stage=received", "stage=stored")))
        summary = module.summarize(records)
        self.assertEqual(len(summary), 3)
        self.assertEqual(summary[0]["stored_entries_including_repeats"], 42)

    def test_already_stored_is_not_counted_as_another_write(self):
        record = module.extract(LINE.replace("stage=received", "stage=already_stored").replace("entries=0", "entries=42"))
        summary = module.summarize([record])[0]
        self.assertEqual(summary["events"], {"already_stored": 1})
        self.assertEqual(summary["stored_entries_including_repeats"], 0)

    def test_quiet_and_request_failure_are_observations_not_completion(self):
        quiet = module.extract(LINE.replace("stage=received", "stage=quiet"))
        failed = module.extract(LINE.replace("flow=initial", "flow=on_demand").replace("stage=received", "stage=request_failed"))
        summary = module.summarize([quiet, failed])
        self.assertEqual(summary[0]["events"], {"quiet": 1})
        self.assertNotIn("stored", summary[0]["last_event_times"])
        self.assertEqual(summary[1]["events"], {"request_failed": 1})
        self.assertIsNone(module.extract(LINE.replace("stage=received", "stage=request_failed")))

    def test_dispatch_and_safe_rejection_categories_are_exported_without_identifiers(self):
        for stage in ("dispatch_started", "dispatch_timeout", "request_rejected_not_connected",
                      "request_rejected_invalid_chat", "request_deferred"):
            line = LINE.replace("flow=initial", "flow=on_demand").replace("stage=received", f"stage={stage}")
            record = module.extract(line)
            self.assertEqual(record["stage"], stage)
            self.assertEqual(set(record), {"time", "version", "account", "flow", "stage",
                                           "sync_type", "percent", "chats", "entries", "elapsed_ms"})

    def test_request_context_and_job_status_accept_only_fixed_safe_fields(self):
        context = ("2026-10-19T21:00:00Z INFO zapfast [HISTORY_REQUEST] version=1 account=4 "
                   "capability=disabled companion=unknown anchor=direct_pn empty=0 archived=1 active=12 reserved=1")
        job = ("2026-10-19T21:00:01Z INFO zapfast [HISTORY_JOB] version=1 account=4 "
               "state=backing_off reason=retry_timeout failure_streak=2 retry_at=1792443600 "
               "completed=0 total=12 excluded=1")
        self.assertEqual(module.extract_request_context(context)["anchor"], "direct_pn")
        self.assertEqual(module.extract_job_status(job)["failure_streak"], 2)
        self.assertIsNone(module.extract_request_context(context + " chat=private"))
        self.assertIsNone(module.extract_job_status(job.replace("reason=retry_timeout", "reason=private")))

    def test_media_status_accepts_only_fixed_aggregate_fields(self):
        media = ("2026-10-19T21:00:02Z INFO zapfast [MEDIA_JOB] version=1 account=4 "
                 "snapshot_at=1792443602 enabled=1 user_paused=0 privacy_paused=0 connected=1 "
                 "runnable=1 phone_runnable=0 "
                 "phone_recovery=0 large_files=0 target_since=1698105600 cdn_pending=25 needs_phone=3 "
                 "downloaded=40 unavailable=2 policy_too_large=1 cancelled=0 schedule_known=1 "
                 "cdn_due=20 phone_due=3 active=2 retry_at=1792443660 phone_quota_retry_at=0 "
                 "bytes=1048576 budget_mib=20480 budget_reached=0")
        record = module.extract_media_status(media)
        self.assertEqual(record["cdn_pending"], 25)
        self.assertEqual(record["bytes"], 1048576)
        self.assertEqual(record["phone_recovery"], 0)
        self.assertEqual(record["schedule_known"], 1)
        self.assertIsNone(module.extract_media_status(media + " chat=private"))
        self.assertIsNone(module.extract_media_status(media.replace("bytes=1048576", "bytes=-2")))
        self.assertIsNone(module.extract_media_status(media.replace("connected=1", "connected=2")))

    def test_history_rows_accepts_only_net_new_aggregate(self):
        rows = ("2026-10-19T21:00:03Z INFO zapfast [HISTORY_ROWS] version=1 account=4 "
                "snapshot_at=1792443603 added=17")
        self.assertEqual(module.extract_history_rows(rows)["added"], 17)
        self.assertIsNone(module.extract_history_rows(rows + " chat=private"))
        self.assertIsNone(module.extract_history_rows(rows.replace("added=17", "added=-1")))


if __name__ == "__main__":
    unittest.main()
