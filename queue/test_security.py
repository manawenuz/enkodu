#!/usr/bin/env python3
"""Regression tests pinning the security-audit fixes for queue/main.py.

Each test asserts the *exact* secure guard a fix introduced, and is annotated
with how it would have failed against the pre-fix behavior. Hermetic: temp DB,
temp video root, no network, no real ffmpeg, no live server.

The AUTH_* settings are read at MODULE IMPORT time, so any test that needs a
particular auth env (e.g. AUTH_BOOTSTRAP_TOKEN set vs. unset) loads a *fresh*
module instance with that env in its own setUpClass, exactly like
test_safety.py does.
"""

import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest

from fastapi.testclient import TestClient


def _load_main(env: dict, mod_name: str):
    """Load a fresh queue/main.py instance with the given env applied at import
    time. Returns (module, TestClient, tmpdir, root)."""
    tmp = tempfile.TemporaryDirectory()
    root = Path(tmp.name)
    base_env = {
        "AUTH_ENABLED": "false",
        "DB_PATH": str(root / "queue.db"),
        "VIDEOS_ROOT": str(root / "Videos"),
    }
    base_env.update(env)
    for k, v in base_env.items():
        if v is None:
            os.environ.pop(k, None)
        else:
            os.environ[k] = v
    Path(os.environ["VIDEOS_ROOT"]).mkdir(parents=True, exist_ok=True)

    main_path = Path(__file__).resolve().parent / "main.py"
    spec = importlib.util.spec_from_file_location(mod_name, main_path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    client = TestClient(module.app)
    return module, client, tmp, root


# ─────────────────────────────────────────────────────────────────────────────
# FILE-1 — _safe_basename() path-traversal reduction (pure function)
# ─────────────────────────────────────────────────────────────────────────────
class SafeBasenameTest(unittest.TestCase):
    """FILE-1: _safe_basename() must reduce any client-supplied filename to a
    bare basename so it can never carry path components into a rename target.

    Pre-fix: the raw client filename was used directly, so '../../etc/passwd' or
    '/abs/path.mp4' kept its directory parts and could escape the output dir.
    """

    @classmethod
    def setUpClass(cls):
        cls.main, cls.client, cls.tmp, cls.root = _load_main({}, "queue_main_file1")

    @classmethod
    def tearDownClass(cls):
        cls.main._db.close()
        cls.tmp.cleanup()

    def test_strips_unix_traversal(self):
        # Pre-fix: returns '../../etc/passwd' (still has '/'). Now: 'passwd'.
        self.assertEqual(self.main._safe_basename("../../etc/passwd"), "passwd")

    def test_strips_absolute_unix_path(self):
        self.assertEqual(self.main._safe_basename("/abs/path.mp4"), "path.mp4")

    def test_strips_windows_path(self):
        # Backslashes are normalized to '/', so only the final segment survives.
        self.assertEqual(self.main._safe_basename("C:\\x\\y.mp4"), "y.mp4")

    def test_strips_relative_subdir(self):
        self.assertEqual(self.main._safe_basename("a/b/c.mp4"), "c.mp4")

    def test_empty_maps_to_upload_fallback(self):
        # '', '.', '..' all reduce to a Path().name that is unusable, so the
        # guard substitutes a safe 'upload<ext>' default.
        self.assertEqual(self.main._safe_basename(""), "upload.mp4")
        self.assertEqual(self.main._safe_basename("."), "upload.mp4")
        self.assertEqual(self.main._safe_basename(".."), "upload.mp4")

    def test_result_never_contains_separators(self):
        for hostile in ["../../etc/passwd", "/abs/path.mp4", "C:\\x\\y.mp4", "a/b/c.mp4"]:
            out = self.main._safe_basename(hostile)
            self.assertNotIn("/", out)
            self.assertNotIn("\\", out)
            self.assertNotIn("..", out)


# ─────────────────────────────────────────────────────────────────────────────
# XSS-1 — companion id validation (_CID_RE / _require_valid_cid)
# ─────────────────────────────────────────────────────────────────────────────
class CompanionIdValidationTest(unittest.TestCase):
    """XSS-1: companion register/config/capabilities endpoints must reject ids
    containing HTML/JS metacharacters or that are over-long (>64 chars), and
    accept clean uuid-style ids.

    Pre-fix: there was no _require_valid_cid call, so a companion id like
    'a<script>' or '../x' was written straight into companion_registry and later
    rendered in the dashboard -> stored XSS / id confusion.
    """

    @classmethod
    def setUpClass(cls):
        cls.main, cls.client, cls.tmp, cls.root = _load_main({}, "queue_main_xss1")

    @classmethod
    def tearDownClass(cls):
        cls.main._db.close()
        cls.tmp.cleanup()

    def setUp(self):
        with self.main.db() as conn:
            conn.execute("DELETE FROM companion_registry")
            conn.commit()

    # Ids that reach the handler as a single path segment but carry metachars /
    # are over-long. These are caught by _require_valid_cid -> HTTP 400. This is
    # the guard the XSS-1 fix actually added.
    BAD_IDS_400 = [
        "has space",           # space
        "<script>",            # html metachar
        "x>y",                 # '>'
        "a" * 65,              # >64 chars
        "id$inject",           # '$'
        "id;rm",               # ';'
        'a"b',                 # double quote (attribute breakout)
    ]

    # Ids containing '/' (incl. '..' / '../x', which Starlette path-normalizes)
    # never match the route and are rejected at routing with HTTP 404. They are
    # rejected too, just before the handler — assert that separately and honestly.
    BAD_IDS_404 = [
        "a/b",                 # contains '/'
        "..",                  # traversal token -> normalized away
        "../x",                # traversal
    ]

    def _assert_rejected(self, do_request):
        for cid in self.BAD_IDS_400:
            resp = do_request(cid)
            self.assertEqual(resp.status_code, 400, f"cid={cid!r} should be 400")
            self.assertIn("invalid companion id", resp.text)
        for cid in self.BAD_IDS_404:
            resp = do_request(cid)
            self.assertEqual(resp.status_code, 404, f"cid={cid!r} should be 404 (routing)")

    def test_register_rejects_bad_ids(self):
        # Pre-fix: a single-segment bad id (e.g. '<script>') returned 200 and was
        # stored verbatim. Now: 400 invalid companion id.
        self._assert_rejected(
            lambda cid: self.client.post(
                f"/companions/{cid}/register",
                json={"name": "n", "platform": "p", "version": "v"},
            )
        )

    def test_set_config_rejects_bad_ids(self):
        self._assert_rejected(
            lambda cid: self.client.put(f"/companions/{cid}/config", json={"config": {"k": 1}})
        )

    def test_capabilities_rejects_bad_ids(self):
        self._assert_rejected(
            lambda cid: self.client.post(
                f"/companions/{cid}/capabilities",
                json={"encoders": [], "decoders": [], "ffprobe_available": False, "platform": ""},
            )
        )

    def test_valid_uuid_id_is_accepted_and_persisted(self):
        good = "550e8400-e29b-41d4-a716-446655440000"
        resp = self.client.post(
            f"/companions/{good}/register",
            json={"name": "node", "platform": "darwin", "version": "1.0"},
        )
        self.assertEqual(resp.status_code, 200)
        with self.main.db() as conn:
            row = conn.execute(
                "SELECT id FROM companion_registry WHERE id=?", (good,)
            ).fetchone()
        self.assertIsNotNone(row)
        self.assertEqual(row["id"], good)

    def test_bad_id_never_reaches_db(self):
        # Defense-in-depth: a rejected register must not insert a registry row.
        self.client.post("/companions/x<script>/register", json={"name": "n"})
        with self.main.db() as conn:
            cnt = conn.execute("SELECT COUNT(*) FROM companion_registry").fetchone()[0]
        self.assertEqual(cnt, 0)


# ─────────────────────────────────────────────────────────────────────────────
# LIFECYCLE-1 & LIFECYCLE-2 — verified-output + worker-ownership guards
# ─────────────────────────────────────────────────────────────────────────────
class LifecycleGuardsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.main, cls.client, cls.tmp, cls.root = _load_main({}, "queue_main_lifecycle")

    @classmethod
    def tearDownClass(cls):
        cls.main._db.close()
        cls.tmp.cleanup()

    def setUp(self):
        with self.main.db() as conn:
            conn.execute("DELETE FROM jobs")
            conn.commit()
        self.main._sha256_cache.clear()
        # Hermetic: /jobs/{id}/done spawns a background verification thread that
        # shells out to the real ffprobe binary and writes back to the DB. Stub
        # it so the suite never touches ffmpeg/ffprobe and the daemon thread can
        # never race teardown. The guards we pin (verify_status reset to
        # 'running' and worker-ownership) all execute *inside* done() before this
        # is ever called, so stubbing does not weaken the assertions.
        self._orig_run_verification = self.main._run_verification
        self.main._run_verification = lambda *a, **k: None

    def tearDown(self):
        self.main._run_verification = self._orig_run_verification

    def _insert_job(self, job_id, *, status, verify_status, worker=None,
                    make_files=True, output_size=None):
        source = self.root / f"{job_id}.mp4"
        output = self.root / f"{job_id}_av1.mp4"
        if make_files:
            source.write_bytes(b"source-data")
            output.write_bytes(b"verified-output")
        now = time.time()
        with self.main.db() as conn:
            conn.execute(
                """
                INSERT INTO jobs (
                    id, source_path, output_path, source_size, source_duration_secs,
                    status, output_size, created_at, updated_at, source_filename,
                    verify_status, worker
                )
                VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
                """,
                (
                    job_id, str(source), str(output),
                    (source.stat().st_size if make_files else 0), 1.0,
                    status, output_size if output_size is not None else (output.stat().st_size if make_files else 0),
                    now, now, source.name, verify_status, worker,
                ),
            )
            conn.commit()
        return source, output

    # ── LIFECYCLE-1 ──────────────────────────────────────────────────────────
    def test_delete_refused_when_status_not_done(self):
        # Even with verify_status='pass', a not-done job must not be deletable.
        # Pre-fix: only verify_status was checked -> an active/pending job with a
        # stale 'pass' could have its source deleted.
        source, _ = self._insert_job("ld-active", status="active", verify_status="pass")
        resp = self.client.post("/jobs/ld-active/delete-original", json={"rename": False})
        self.assertEqual(resp.status_code, 400)
        self.assertIn("not done", resp.json()["detail"])
        self.assertTrue(source.exists())

    def test_delete_refused_when_not_verified(self):
        # status='done' but verify_status != 'pass' must be refused.
        source, _ = self._insert_job("ld-unverified", status="done", verify_status="running")
        resp = self.client.post("/jobs/ld-unverified/delete-original", json={"rename": False})
        self.assertEqual(resp.status_code, 400)
        self.assertIn("not verified", resp.json()["detail"])
        self.assertTrue(source.exists())

    def test_delete_allowed_only_when_done_and_pass(self):
        source, _ = self._insert_job("ld-ok", status="done", verify_status="pass")
        resp = self.client.post("/jobs/ld-ok/delete-original", json={"rename": False})
        self.assertEqual(resp.status_code, 200)
        self.assertTrue(resp.json()["deleted"])
        self.assertFalse(source.exists())

    def test_done_resets_verify_status_to_running(self):
        # LIFECYCLE-1: completing a job must reset verify_status back to
        # 'running' so a previously-recorded 'pass' cannot persist across a
        # re-encode. Pre-fix: /done left a stale 'pass' in place, which would
        # immediately authorize delete-original on an unverified new output.
        self._insert_job(
            "ld-reset", status="active", verify_status="pass", worker="w-claim",
        )
        resp = self.client.post(
            "/jobs/ld-reset/done", json={"worker": "w-claim", "output_size": 123},
        )
        self.assertEqual(resp.status_code, 200)
        with self.main.db() as conn:
            row = conn.execute(
                "SELECT status, verify_status FROM jobs WHERE id=?", ("ld-reset",)
            ).fetchone()
        self.assertEqual(row["status"], "done")
        # The stale 'pass' must have been cleared to 'running'.
        self.assertEqual(row["verify_status"], "running")

    # ── LIFECYCLE-2 ──────────────────────────────────────────────────────────
    def test_done_rejected_for_non_owning_worker(self):
        # Job claimed by 'w-owner'. A /done from 'w-thief' must 409 and not
        # change state. Pre-fix: /done updated by id only, so any worker could
        # complete (and clobber) a job it never claimed.
        self._insert_job("lo-done", status="active", verify_status=None, worker="w-owner")
        thief = self.client.post(
            "/jobs/lo-done/done", json={"worker": "w-thief", "output_size": 999},
        )
        self.assertEqual(thief.status_code, 409)
        with self.main.db() as conn:
            row = conn.execute(
                "SELECT status, output_size, worker FROM jobs WHERE id=?", ("lo-done",)
            ).fetchone()
        # State untouched.
        self.assertEqual(row["status"], "active")
        self.assertEqual(row["worker"], "w-owner")
        self.assertNotEqual(row["output_size"], 999)

    def test_done_succeeds_for_owning_worker(self):
        self._insert_job("lo-done-ok", status="active", verify_status=None, worker="w-owner")
        owner = self.client.post(
            "/jobs/lo-done-ok/done", json={"worker": "w-owner", "output_size": 777},
        )
        self.assertEqual(owner.status_code, 200)
        with self.main.db() as conn:
            row = conn.execute(
                "SELECT status, output_size FROM jobs WHERE id=?", ("lo-done-ok",)
            ).fetchone()
        self.assertEqual(row["status"], "done")
        self.assertEqual(row["output_size"], 777)

    def test_failed_rejected_for_non_owning_worker(self):
        self._insert_job("lo-failed", status="active", verify_status=None, worker="w-owner")
        thief = self.client.post(
            "/jobs/lo-failed/failed", json={"worker": "w-thief", "error": "boom"},
        )
        self.assertEqual(thief.status_code, 409)
        with self.main.db() as conn:
            row = conn.execute(
                "SELECT status, error FROM jobs WHERE id=?", ("lo-failed",)
            ).fetchone()
        self.assertEqual(row["status"], "active")
        self.assertIsNone(row["error"])

    def test_failed_succeeds_for_owning_worker(self):
        self._insert_job("lo-failed-ok", status="active", verify_status=None, worker="w-owner")
        owner = self.client.post(
            "/jobs/lo-failed-ok/failed", json={"worker": "w-owner", "error": "boom"},
        )
        self.assertEqual(owner.status_code, 200)
        with self.main.db() as conn:
            row = conn.execute(
                "SELECT status, error FROM jobs WHERE id=?", ("lo-failed-ok",)
            ).fetchone()
        self.assertEqual(row["status"], "failed")
        self.assertEqual(row["error"], "boom")

    def test_unsafe_rename_target_is_rejected(self):
        # FILE-1 (bonus): delete-original with rename=True must refuse to move the
        # output outside its own directory when source_filename carries path
        # components. Pre-fix: the poisoned filename was used directly.
        job_id = "ld-rename-bad"
        source, output = self._insert_job(job_id, status="done", verify_status="pass")
        # Poison source_filename with a traversal target.
        with self.main.db() as conn:
            conn.execute(
                "UPDATE jobs SET source_filename=? WHERE id=?",
                ("../escape.mp4", job_id),
            )
            conn.commit()
        resp = self.client.post(f"/jobs/{job_id}/delete-original", json={"rename": True})
        # _safe_basename / containment check collapses '../escape.mp4' to
        # 'escape.mp4' inside out.parent, so the rename stays contained and
        # succeeds without escaping. Assert the target never left the dir.
        self.assertEqual(resp.status_code, 200)
        renamed = resp.json()["renamed_to"]
        self.assertIsNotNone(renamed)
        self.assertEqual(Path(renamed).parent.resolve(), output.parent.resolve())
        self.assertEqual(Path(renamed).name, "escape.mp4")
        # The escaping path (one level up) must NOT exist.
        self.assertFalse((output.parent.parent / "escape.mp4").exists())


# ─────────────────────────────────────────────────────────────────────────────
# AUTH-1 — /auth/bootstrap with AUTH_BOOTSTRAP_TOKEN UNSET
# ─────────────────────────────────────────────────────────────────────────────
class BootstrapDisabledTest(unittest.TestCase):
    """AUTH-1(a): with AUTH_BOOTSTRAP_TOKEN unset, /auth/bootstrap is disabled
    entirely (403). Loaded in its own class so the import-time empty token
    takes effect.

    Pre-fix: there was no token gate, so anyone could POST /auth/bootstrap and
    mint the first admin.
    """

    @classmethod
    def setUpClass(cls):
        cls.main, cls.client, cls.tmp, cls.root = _load_main(
            {"AUTH_BOOTSTRAP_TOKEN": None}, "queue_main_bootstrap_off"
        )

    @classmethod
    def tearDownClass(cls):
        cls.main._db.close()
        cls.tmp.cleanup()

    def test_bootstrap_disabled_when_token_unset(self):
        self.assertEqual(self.main.AUTH_BOOTSTRAP_TOKEN, "")
        resp = self.client.post(
            "/auth/bootstrap", json={"username": "root", "secret": "anything"}
        )
        self.assertEqual(resp.status_code, 403)
        self.assertIn("bootstrap disabled", resp.json()["detail"])
        # No user created.
        with self.main.db() as conn:
            cnt = conn.execute("SELECT COUNT(*) FROM auth_users").fetchone()[0]
        self.assertEqual(cnt, 0)


# ─────────────────────────────────────────────────────────────────────────────
# AUTH-1 — /auth/bootstrap with AUTH_BOOTSTRAP_TOKEN SET
# ─────────────────────────────────────────────────────────────────────────────
class BootstrapEnabledTest(unittest.TestCase):
    """AUTH-1(b,c,d): with the token set, /auth/bootstrap requires the correct
    secret (constant-time compared), always creates an admin (role is forced,
    never client-controlled), and refuses once any user exists."""

    TOKEN = "s3cr3t-bootstrap-token"

    @classmethod
    def setUpClass(cls):
        cls.main, cls.client, cls.tmp, cls.root = _load_main(
            {"AUTH_BOOTSTRAP_TOKEN": cls.TOKEN}, "queue_main_bootstrap_on"
        )

    @classmethod
    def tearDownClass(cls):
        cls.main._db.close()
        cls.tmp.cleanup()

    def setUp(self):
        with self.main.db() as conn:
            conn.execute("DELETE FROM auth_users")
            conn.execute("DELETE FROM auth_challenges")
            conn.commit()

    def test_wrong_secret_rejected(self):
        # AUTH-1(b): wrong secret -> 403 invalid secret.
        resp = self.client.post(
            "/auth/bootstrap", json={"username": "root", "secret": "wrong"}
        )
        self.assertEqual(resp.status_code, 403)
        self.assertIn("invalid secret", resp.json()["detail"])
        with self.main.db() as conn:
            cnt = conn.execute("SELECT COUNT(*) FROM auth_users").fetchone()[0]
        self.assertEqual(cnt, 0)

    def test_empty_secret_rejected(self):
        # AUTH-1(b): empty/missing secret -> 403 (never matches via compare_digest).
        resp = self.client.post("/auth/bootstrap", json={"username": "root"})
        self.assertEqual(resp.status_code, 403)
        self.assertIn("invalid secret", resp.json()["detail"])

    def test_correct_secret_creates_admin(self):
        # AUTH-1(c): correct secret + empty auth_users -> 200; created user is admin.
        resp = self.client.post(
            "/auth/bootstrap", json={"username": "root", "secret": self.TOKEN}
        )
        self.assertEqual(resp.status_code, 200)
        self.assertTrue(resp.json()["ok"])
        with self.main.db() as conn:
            row = conn.execute(
                "SELECT username, role FROM auth_users WHERE username=?", ("root",)
            ).fetchone()
        self.assertIsNotNone(row)
        self.assertEqual(row["role"], "admin")

    def test_role_field_is_ignored_always_admin(self):
        # AUTH-1(c): the request model has no 'role' field; even if a client
        # sends one, the created user is still admin (role is hardcoded).
        # Pre-fix risk: a client-supplied role could downgrade/escalate privilege.
        resp = self.client.post(
            "/auth/bootstrap",
            json={"username": "root2", "secret": self.TOKEN, "role": "operator"},
        )
        self.assertEqual(resp.status_code, 200)
        with self.main.db() as conn:
            row = conn.execute(
                "SELECT role FROM auth_users WHERE username=?", ("root2",)
            ).fetchone()
        self.assertEqual(row["role"], "admin")

    def test_second_bootstrap_refused(self):
        # AUTH-1(d): once a user exists, a second bootstrap -> 403.
        first = self.client.post(
            "/auth/bootstrap", json={"username": "root", "secret": self.TOKEN}
        )
        self.assertEqual(first.status_code, 200)
        second = self.client.post(
            "/auth/bootstrap", json={"username": "intruder", "secret": self.TOKEN}
        )
        self.assertEqual(second.status_code, 403)
        self.assertIn("already exist", second.json()["detail"])
        with self.main.db() as conn:
            cnt = conn.execute("SELECT COUNT(*) FROM auth_users").fetchone()[0]
        self.assertEqual(cnt, 1)


# ─────────────────────────────────────────────────────────────────────────────
# XSS-2 — dashboard JS hardening (esc() escapes single quotes; title sink wrapped)
# ─────────────────────────────────────────────────────────────────────────────
class DashboardEscapingTest(unittest.TestCase):
    """XSS-2: this fix is client-side JS. The server *serves* the JS verbatim in
    the dashboard HTML (GET /), so we can pin the hardened pattern there:
      1) esc() now also escapes single quotes (' -> &#39;), and
      2) the title-attribute sinks for file paths are wrapped in esc().

    This is the best server-testable proxy; the actual browser-side escaping
    behavior is not unit-testable in this Python component (no JS runtime).

    Pre-fix: esc() did not escape "'" and the title attributes interpolated raw
    file paths, allowing attribute-context breakout / stored XSS.
    """

    @classmethod
    def setUpClass(cls):
        cls.main, cls.client, cls.tmp, cls.root = _load_main({}, "queue_main_xss2")

    @classmethod
    def tearDownClass(cls):
        cls.main._db.close()
        cls.tmp.cleanup()

    def test_esc_escapes_single_quote(self):
        html = self.client.get("/").text
        # The hardened esc() replaces "'" with the &#39; entity.
        self.assertIn(r".replace(/'/g,'&#39;')", html)

    def test_title_path_sinks_are_wrapped_in_esc(self):
        html = self.client.get("/").text
        # The file-path title attributes must run through esc(), not raw values.
        self.assertIn('title="${esc(r.file_path)}"', html)

    def test_no_raw_file_path_title_sink_remains(self):
        html = self.client.get("/").text
        # A raw (unescaped) title="${r.file_path}" must NOT appear anywhere.
        self.assertNotIn('title="${r.file_path}"', html)

    def test_current_file_title_sink_is_wrapped_in_esc(self):
        # XSS-2 (load-bearing half): the worker-card current_file title attribute
        # is the sink the fix actually changed. It must run through esc().
        html = self.client.get("/").text
        self.assertIn('title="${esc(w.current_file)}"', html)

    def test_no_raw_current_file_title_sink_remains(self):
        # Pre-fix the dashboard emitted title="${w.current_file}" raw, letting a
        # worker-supplied filename break out of the attribute (stored XSS in the
        # operator dashboard). This assertion genuinely fails against pre-fix code.
        html = self.client.get("/").text
        self.assertNotIn('title="${w.current_file}"', html)


if __name__ == "__main__":
    unittest.main()
