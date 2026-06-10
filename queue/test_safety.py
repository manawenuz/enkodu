#!/usr/bin/env python3
"""Focused safety hardening tests for queue/main.py."""

import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest

from fastapi.testclient import TestClient


class QueueSafetyTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.root = Path(cls.tmp.name)
        os.environ["AUTH_ENABLED"] = "false"
        os.environ["DB_PATH"] = str(cls.root / "queue.db")
        os.environ["VIDEOS_ROOT"] = str(cls.root / "Videos")
        Path(os.environ["VIDEOS_ROOT"]).mkdir(parents=True, exist_ok=True)

        main_path = Path(__file__).resolve().parent / "main.py"
        spec = importlib.util.spec_from_file_location("queue_main_safety_test", main_path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)

        cls.main = module
        cls.client = TestClient(module.app)

    @classmethod
    def tearDownClass(cls):
        cls.main._db.close()
        cls.tmp.cleanup()

    def setUp(self):
        with self.main.db() as conn:
            conn.execute("DELETE FROM jobs")
            conn.execute("DELETE FROM telemetry")
            conn.commit()
        self.main._sha256_cache.clear()

    def _create_job(self, job_id: str, verify_status: str | None = "pass", status: str = "done"):
        source = self.root / f"{job_id}.mp4"
        output = self.root / f"{job_id}_av1.mp4"
        source.write_bytes(b"source-data")
        output.write_bytes(b"verified-output")
        now = time.time()
        with self.main.db() as conn:
            conn.execute(
                """
                INSERT INTO jobs (
                    id, source_path, output_path, source_size, source_duration_secs,
                    status, output_size, created_at, updated_at, source_filename,
                    verify_status
                )
                VALUES (?,?,?,?,?,?,?,?,?,?,?)
                """,
                (
                    job_id, str(source), str(output), source.stat().st_size, 1.0,
                    status, output.stat().st_size, now, now, source.name,
                    verify_status,
                ),
            )
            conn.commit()
        return source, output

    def test_output_download_requires_verify_pass(self):
        self._create_job("output-fail", verify_status="fail")

        denied = self.client.get("/jobs/output-fail/output")
        self.assertEqual(denied.status_code, 404)

        self._create_job("output-pass", verify_status="pass")
        allowed = self.client.get("/jobs/output-pass/output")
        self.assertEqual(allowed.status_code, 200)
        self.assertEqual(allowed.content, b"verified-output")

    def test_checksum_output_requires_verify_pass(self):
        self._create_job("checksum-fail", verify_status=None)

        denied = self.client.get("/jobs/checksum-fail/checksum")
        self.assertEqual(denied.status_code, 404)

        self._create_job("checksum-pass", verify_status="pass")
        allowed = self.client.get("/jobs/checksum-pass/checksum")
        self.assertEqual(allowed.status_code, 200)
        self.assertIn("source_sha256", allowed.json())
        self.assertIn("output_sha256", allowed.json())

    def test_delete_original_requires_verify_pass(self):
        source, _ = self._create_job("delete-fail", verify_status="fail")

        denied = self.client.post("/jobs/delete-fail/delete-original", json={"rename": False})
        self.assertEqual(denied.status_code, 400)
        self.assertTrue(source.exists())

        source, _ = self._create_job("delete-pass", verify_status="pass")
        allowed = self.client.post("/jobs/delete-pass/delete-original", json={"rename": False})
        self.assertEqual(allowed.status_code, 200)
        self.assertTrue(allowed.json()["deleted"])
        self.assertFalse(source.exists())

    def test_bulk_delete_original_requires_verify_pass_per_job(self):
        fail_source, _ = self._create_job("bulk-fail", verify_status="running")
        pass_source, _ = self._create_job("bulk-pass", verify_status="pass")

        resp = self.client.post(
            "/jobs/bulk-delete-original",
            json={"ids": ["bulk-fail", "bulk-pass"], "rename": False},
        )
        self.assertEqual(resp.status_code, 200)
        results = {item["id"]: item for item in resp.json()["results"]}
        self.assertFalse(results["bulk-fail"]["ok"])
        self.assertTrue(results["bulk-pass"]["ok"])
        self.assertTrue(fail_source.exists())
        self.assertFalse(pass_source.exists())

    def test_telemetry_guardrails_reject_secret_and_oversized_fields(self):
        ok = self.client.post("/telemetry", json={"event_type": "download", "event_detail": "range retry"})
        self.assertEqual(ok.status_code, 200)

        secret = self.client.post(
            "/telemetry",
            json={"event_type": "download", "event_detail": "Authorization: Bearer abc123"},
        )
        self.assertEqual(secret.status_code, 400)

        too_long = self.client.post("/telemetry", json={"event_type": "x" * 81})
        self.assertEqual(too_long.status_code, 400)

    def test_telemetry_guardrails_reject_path_heavy_detail(self):
        resp = self.client.post(
            "/telemetry",
            json={
                "event_type": "error",
                "event_detail": "copy failed /Users/alice/input.mp4 -> /mnt/pool/output.mp4",
            },
        )
        self.assertEqual(resp.status_code, 400)


if __name__ == "__main__":
    unittest.main()
