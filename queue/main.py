"""
Queue service — runs on TrueNAS via Docker Compose.
Workers poll GET /jobs/next to claim work; results posted back via /jobs/{id}/done|failed.
"""

import hashlib, json, os, sqlite3, threading, subprocess, uuid, time, logging
import urllib.request as _ureq
from contextlib import contextmanager
from pathlib import Path
from urllib.parse import quote, unquote
from fastapi import FastAPI, Header, HTTPException, Request
from fastapi.responses import JSONResponse, HTMLResponse, StreamingResponse
from typing import Optional
from pydantic import BaseModel

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger(__name__)

def _fix_mojibake(s: str) -> str:
    """Fix UTF-8 text that was decoded as Latin-1 (common Mac companion encoding bug)."""
    if not s:
        return s
    try:
        return s.encode("latin-1").decode("utf-8")
    except (UnicodeEncodeError, UnicodeDecodeError):
        return s

_KPOP_ADJ = [
    "Starry","Neon","Crystal","Velvet","Lunar","Solar","Dreamy","Cosmic",
    "Prism","Silky","Aurora","Ivory","Golden","Silver","Misty","Blazing",
    "Twilight","Midnight","Dewy","Radiant","Glitter","Sakura","Cherry",
    "Shimmer","Frosty","Sparkling","Pastel","Electric","Rosy","Azure",
]
_KPOP_NOUN = [
    "Star","Moon","Beat","Dance","Wave","Bloom","Dream","Nova",
    "Glow","Rise","Burst","Pulse","Shine","Flow","Storm","Spark",
    "Idol","Stan","Echo","Hana","Miso","Sora","Yuki","Rina",
    "Berry","Honey","Candy","Boba","Pearl","Gem",
]
_KPOP_COLORS = ["#f48fb1","#ce93d8","#80cbc4","#b39ddb","#ffcc80","#a5d6a7","#ef9a9a","#90caf9"]

def _kpop_name(ip: str) -> str:
    h = int(hashlib.md5(ip.encode()).hexdigest(), 16)
    return f"{_KPOP_ADJ[h % len(_KPOP_ADJ)]}{_KPOP_NOUN[(h >> 8) % len(_KPOP_NOUN)]}"

def _kpop_color(name: str) -> str:
    h = int(hashlib.md5(name.encode()).hexdigest(), 16)
    return _KPOP_COLORS[h % len(_KPOP_COLORS)]

_live: dict = {}
_workers: dict = {}

def _worker_update(name: str, status: str, job_id: str = None, filename: str = None):
    _workers[name] = {"last_seen": time.time(), "status": status,
                      "current_job": job_id, "current_file": filename}

TG_TOKEN = os.getenv("TELEGRAM_BOT_TOKEN", "")
TG_CHAT  = os.getenv("TELEGRAM_CHAT_ID", "")

def _tg(text: str):
    if not TG_TOKEN or not TG_CHAT:
        return
    def _send():
        for cid in TG_CHAT.split(","):
            cid = cid.strip()
            if not cid:
                continue
            try:
                body = json.dumps({"chat_id": cid, "text": text, "parse_mode": "HTML"}).encode()
                req = _ureq.Request(
                    f"https://api.telegram.org/bot{TG_TOKEN}/sendMessage",
                    data=body, headers={"Content-Type": "application/json"}
                )
                _ureq.urlopen(req, timeout=10)
            except Exception as e:
                log.warning("Telegram send failed (chat %s): %s", cid, e)
    threading.Thread(target=_send, daemon=True).start()

VIDEOS_ROOT  = Path(os.getenv("VIDEOS_ROOT", "/data/Videos"))
NAS_UNC_ROOT = os.getenv("NAS_UNC_ROOT", r"\\172.16.81.137\yulia")
DB_PATH      = os.getenv("DB_PATH", "/data/.transcode/queue.db")
SCAN_INTERVAL = int(os.getenv("SCAN_INTERVAL", "300"))
VIDEO_EXTS   = {".mp4", ".mkv", ".avi", ".mov", ".ts", ".m2ts", ".wmv"}
FFPROBE      = os.getenv("FFPROBE", "ffprobe")
STALL_TIMEOUT = int(os.getenv("STALL_TIMEOUT", "900"))

_CONTROL_PATH = Path(DB_PATH).parent / "control.json"

def _load_control() -> dict:
    try:
        if _CONTROL_PATH.exists():
            data = json.loads(_CONTROL_PATH.read_text())
            if data.get("command") in ("run", "drain", "stop"):
                return data
    except Exception:
        pass
    return {"command": "run"}

def _save_control(cmd: str):
    try:
        _CONTROL_PATH.write_text(json.dumps({"command": cmd}))
    except Exception as e:
        log.warning("Failed to persist control state: %s", e)

_control: dict = _load_control()

# ── DB ────────────────────────────────────────────────────────────────────────

def db_connect():
    os.makedirs(os.path.dirname(DB_PATH), exist_ok=True)
    conn = sqlite3.connect(DB_PATH, check_same_thread=False)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.row_factory = sqlite3.Row
    return conn

def init_db(conn):
    conn.execute("""
        CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            source_path TEXT UNIQUE,
            output_path TEXT,
            source_unc TEXT,
            output_unc TEXT,
            source_size INTEGER,
            source_duration_secs REAL,
            status TEXT DEFAULT 'pending',
            worker TEXT,
            percent REAL DEFAULT 0,
            fps REAL DEFAULT 0,
            speed TEXT DEFAULT '',
            output_size INTEGER,
            error TEXT,
            created_at REAL,
            updated_at REAL
        )
    """)
    conn.execute("""
        CREATE TABLE IF NOT EXISTS clients (
            ip   TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            color TEXT NOT NULL,
            first_seen REAL,
            last_seen  REAL,
            uploads INTEGER DEFAULT 0
        )
    """)
    conn.commit()
    for col in [
        "ADD COLUMN priority INTEGER DEFAULT 0",
        "ADD COLUMN source_filename TEXT",
        "ADD COLUMN verify_status TEXT",
        "ADD COLUMN verify_detail TEXT",
        "ADD COLUMN source_meta TEXT",
        "ADD COLUMN output_meta TEXT",
        "ADD COLUMN verify_checks TEXT",
        "ADD COLUMN client_name TEXT",
        "ADD COLUMN client_path TEXT",
    ]:
        try:
            conn.execute(f"ALTER TABLE jobs {col}")
            conn.commit()
        except sqlite3.OperationalError:
            pass
    for col in [
        "ADD COLUMN weight INTEGER DEFAULT 5",
        "ADD COLUMN queue_manifest TEXT",
    ]:
        try:
            conn.execute(f"ALTER TABLE clients {col}")
            conn.commit()
        except sqlite3.OperationalError:
            pass

def init_settings(conn):
    conn.execute("""
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )
    """)
    conn.commit()
    defaults = {
        "min_size_mb": "0",
        "min_height": "0",
        "min_bitrate_kbps": "0",
        "skip_hevc": "true",
        "skip_av1": "true",
    }
    for k, v in defaults.items():
        try:
            conn.execute("INSERT OR IGNORE INTO settings (key, value) VALUES (?,?)", (k, v))
            conn.commit()
        except sqlite3.OperationalError:
            pass

def _get_setting(key: str, default: str = "") -> str:
    with db() as conn:
        row = conn.execute("SELECT value FROM settings WHERE key=?", (key,)).fetchone()
    return row["value"] if row else default

def _all_settings() -> dict:
    with db() as conn:
        rows = conn.execute("SELECT key, value FROM settings").fetchall()
    return {r["key"]: r["value"] for r in rows}

_db = db_connect()
init_db(_db)
init_settings(_db)
_db_lock = threading.Lock()

@contextmanager
def db():
    with _db_lock:
        yield _db

# ── scanner ───────────────────────────────────────────────────────────────────

def probe_video(path: str) -> dict:
    try:
        r = subprocess.run(
            [FFPROBE, "-v", "error", "-show_streams", "-show_format", "-of", "json", path],
            capture_output=True, text=True, timeout=60
        )
        data = json.loads(r.stdout)
    except Exception:
        return {}
    fmt = data.get("format", {})
    streams = data.get("streams", [])
    video = next((s for s in streams if s.get("codec_type") == "video"), {})
    audio_list = [s for s in streams if s.get("codec_type") == "audio"]

    fps = 0.0
    rfr = video.get("r_frame_rate", "")
    if "/" in rfr:
        n, d = rfr.split("/", 1)
        try:
            fps = float(n) / float(d) if float(d) else 0.0
        except ValueError:
            pass

    duration = 0.0
    try:
        duration = float(fmt.get("duration") or video.get("duration") or 0)
    except (ValueError, TypeError):
        pass

    nb_frames = 0
    try:
        nb_frames = int(video.get("nb_frames") or 0)
    except (ValueError, TypeError):
        pass
    if not nb_frames and fps and duration:
        nb_frames = int(duration * fps)

    return {
        "duration": round(duration, 3),
        "bitrate": int(fmt.get("bit_rate") or 0),
        "video_codec": video.get("codec_name", ""),
        "width": int(video.get("width") or 0),
        "height": int(video.get("height") or 0),
        "fps": round(fps, 3),
        "frames": nb_frames,
        "pix_fmt": video.get("pix_fmt", ""),
        "audio_codecs": [a.get("codec_name", "") for a in audio_list],
        "stream_count": len(streams),
    }

def get_duration(path: str) -> float:
    return probe_video(path).get("duration", 0.0)

def nas_unc(local_path: Path) -> str:
    rel = local_path.relative_to(VIDEOS_ROOT.parent)
    return NAS_UNC_ROOT + "\\" + str(rel).replace("/", "\\")

def scan_videos():
    if _get_setting("nas_drain", "false") == "true":
        log.info("NAS scan paused (nas_drain=true) — skipping")
        return 0
    log.info("Scanning %s ...", VIDEOS_ROOT)
    min_size    = int(_get_setting("min_size_mb", "0")) * 1_000_000
    min_height  = int(_get_setting("min_height", "0"))
    min_bitrate = int(_get_setting("min_bitrate_kbps", "0")) * 1000
    skip_hevc   = _get_setting("skip_hevc", "true") == "true"
    skip_av1    = _get_setting("skip_av1",  "true") == "true"

    added = 0
    for path in VIDEOS_ROOT.rglob("*"):
        if path.suffix.lower() not in VIDEO_EXTS:
            continue
        if "_av1" in path.stem:
            continue

        output_path = path.with_stem(path.stem + "_av1").with_suffix(".mp4")
        if output_path.exists():
            continue

        with db() as conn:
            if conn.execute("SELECT id FROM jobs WHERE source_path=?", (str(path),)).fetchone():
                continue

        size = path.stat().st_size
        if min_size and size < min_size:
            log.debug("Skipping %s (%.1f MB < %d MB min)", path.name, size/1e6, min_size//1_000_000)
            continue

        meta  = probe_video(str(path))
        codec = meta.get("video_codec", "")
        if skip_av1 and codec == "av1":
            log.debug("Skipping %s (already av1)", path.name)
            continue
        if skip_hevc and codec == "hevc":
            log.debug("Skipping %s (already hevc)", path.name)
            continue

        height = meta.get("height", 0)
        if min_height and height and height < min_height:
            log.debug("Skipping %s (%dp < %dp min)", path.name, height, min_height)
            continue

        bitrate = meta.get("bitrate", 0)
        if min_bitrate and bitrate and bitrate < min_bitrate:
            log.debug("Skipping %s (%d kbps < %d kbps min)", path.name, bitrate//1000, min_bitrate//1000)
            continue

        duration = meta.get("duration", 0.0)
        job_id = str(uuid.uuid4())
        now = time.time()

        with db() as conn:
            try:
                conn.execute("""
                    INSERT INTO jobs (id, source_path, output_path, source_unc, output_unc,
                        source_size, source_duration_secs, status, source_meta, client_name, created_at, updated_at)
                    VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
                """, (
                    job_id, str(path), str(output_path),
                    nas_unc(path), nas_unc(output_path),
                    size, duration, "pending", json.dumps(meta), "NAS", now, now
                ))
                conn.commit()
                added += 1
                log.info("Queued: %s (%.1f GB, %.0fs, %s)", path.name, size/1e9, duration, codec)
            except sqlite3.IntegrityError:
                pass

    log.info("Scan complete — %d new jobs added", added)
    return added

def _backfill_nas_origin():
    with db() as conn:
        n = conn.execute(
            "UPDATE jobs SET client_name='NAS' WHERE client_name IS NULL OR client_name=''"
        ).rowcount
        conn.commit()
    if n:
        log.info("Backfilled 'NAS' origin on %d jobs", n)

def _stall_watchdog():
    while True:
        time.sleep(120)
        try:
            cutoff = time.time() - STALL_TIMEOUT
            with db() as conn:
                stalled = conn.execute(
                    "SELECT id, worker, source_path FROM jobs WHERE status='active' AND updated_at < ?",
                    (cutoff,)
                ).fetchall()
                for row in stalled:
                    live = _live.get(row["id"])
                    if live and live.get("updated_at", 0) > cutoff:
                        continue
                    conn.execute(
                        "UPDATE jobs SET status='pending', worker=NULL, error=NULL, percent=0, updated_at=? WHERE id=?",
                        (time.time(), row["id"])
                    )
                    fname = Path(row["source_path"]).name if row["source_path"] else row["id"]
                    log.warning("Stall watchdog: requeued %s (worker=%s)", fname, row["worker"])
                    _tg(f"⚠️ <b>Stall detected</b>  {fname}\n🖥 {row['worker']} — requeued")
                    _live.pop(row["id"], None)
                conn.commit()
        except Exception as e:
            log.error("Stall watchdog error: %s", e)

def scanner_loop():
    time.sleep(10)
    _backfill_nas_origin()
    while True:
        try:
            scan_videos()
        except Exception as e:
            log.error("Scanner error: %s", e)
        time.sleep(SCAN_INTERVAL)

# ── FastAPI ───────────────────────────────────────────────────────────────────

app = FastAPI(title="Yulia AV1 Queue")

@app.on_event("startup")
def startup():
    threading.Thread(target=scanner_loop, daemon=True).start()
    threading.Thread(target=_stall_watchdog, daemon=True).start()

# ── endpoints ─────────────────────────────────────────────────────────────────

@app.get("/api/myip")
async def debug_ip(request: Request):
    return {
        "remote_addr": request.client.host if request.client else None,
        "x_forwarded_for": request.headers.get("x-forwarded-for"),
        "x_real_ip": request.headers.get("x-real-ip"),
    }

def _get_or_create_client(conn, ip: str) -> tuple[str, str]:
    row = conn.execute("SELECT name, color FROM clients WHERE ip=?", (ip,)).fetchone()
    if row:
        conn.execute("UPDATE clients SET last_seen=? WHERE ip=?", (time.time(), ip))
        return row["name"], row["color"]
    name  = _kpop_name(ip)
    color = _kpop_color(name)
    conn.execute(
        "INSERT INTO clients (ip, name, color, first_seen, last_seen, uploads) VALUES (?,?,?,?,?,?)",
        (ip, name, color, time.time(), time.time(), 0)
    )
    log.info("New client: %s → %s (%s)", ip, name, color)
    return name, color

def _pick_weighted_client(conn) -> str | None:
    """Weighted fair queuing: pick the client most behind its target share."""
    pending_rows = conn.execute("""
        SELECT client_name, COUNT(*) as n
        FROM jobs WHERE status='pending' AND client_name IS NOT NULL
        GROUP BY client_name
    """).fetchall()
    available = {r["client_name"]: r["n"] for r in pending_rows if r["n"] > 0}
    if not available:
        return None
    if len(available) == 1:
        return next(iter(available))

    weight_rows = conn.execute("SELECT name, weight FROM clients").fetchall()
    weights = {r["name"]: max(1, r["weight"] or 5) for r in weight_rows}
    for name in available:
        if name not in weights:
            weights[name] = 5

    total_weight = sum(weights.get(c, 5) for c in available)
    targets = {c: weights.get(c, 5) / total_weight for c in available}

    recent_rows = conn.execute("""
        SELECT client_name, COUNT(*) as n FROM (
            SELECT client_name FROM jobs
            WHERE status IN ('active','done','failed') AND client_name IS NOT NULL
            ORDER BY updated_at DESC LIMIT 50
        ) GROUP BY client_name
    """).fetchall()
    recent = {r["client_name"]: r["n"] for r in recent_rows}
    recent_total = sum(recent.values()) or 1

    deficits = {c: targets[c] - recent.get(c, 0) / recent_total for c in available}
    return max(deficits, key=deficits.get)

@app.get("/clients")
def list_clients():
    with db() as conn:
        rows = conn.execute("SELECT * FROM clients ORDER BY last_seen DESC").fetchall()
    result = []
    for r in rows:
        d = dict(r)
        manifest = d.get("queue_manifest")
        d["queued_count"] = len(json.loads(manifest)) if manifest else 0
        result.append(d)
    return result

@app.post("/clients/weights")
async def set_client_weights(request: Request):
    data = await request.json()   # {"ClientName": weight_int, ...}
    with db() as conn:
        for name, w in data.items():
            conn.execute("UPDATE clients SET weight=? WHERE name=?",
                         (max(1, min(100, int(w))), name))
        conn.commit()
    return {"ok": True}

@app.post("/clients/queue-manifest")
async def set_queue_manifest(request: Request):
    ip = (
        request.headers.get("X-Real-IP")
        or (request.headers.get("X-Forwarded-For", "").split(",")[0])
        or (request.client.host if request.client else "unknown")
    ).strip()
    data = await request.json()
    files = data.get("files", [])
    with db() as conn:
        client_name, _ = _get_or_create_client(conn, ip)
        conn.execute("UPDATE clients SET queue_manifest=?, last_seen=? WHERE ip=?",
                     (json.dumps(files), time.time(), ip))
        conn.commit()
    return {"ok": True, "client": client_name, "queued": len(files)}

@app.get("/status")
def status():
    with db() as conn:
        rows = conn.execute("SELECT status, COUNT(*) as n FROM jobs GROUP BY status").fetchall()
    counts = {r["status"]: r["n"] for r in rows}
    return {
        "ok": True,
        "pending": counts.get("pending", 0),
        "active":  counts.get("active",  0),
        "done":    counts.get("done",    0),
        "failed":  counts.get("failed",  0),
    }

@app.get("/workers")
def list_workers():
    now = time.time()
    return [{"name": n, **w, "online": (now - w["last_seen"]) < 120}
            for n, w in _workers.items()]

class HeartbeatReq(BaseModel):
    status: str = "idle"
    current_job: Optional[str] = None
    current_file: Optional[str] = None

@app.post("/workers/{worker_id}/heartbeat")
def worker_heartbeat(worker_id: str, req: HeartbeatReq):
    _worker_update(worker_id, req.status, req.current_job, req.current_file)
    return {"ok": True}

@app.get("/jobs/next")
def next_job(worker: str = "unknown"):
    with db() as conn:
        chosen_client = _pick_weighted_client(conn)
        if chosen_client:
            row = conn.execute(
                "SELECT * FROM jobs WHERE status='pending' AND client_name=?"
                " ORDER BY priority DESC, source_size DESC LIMIT 1",
                (chosen_client,)
            ).fetchone()
        else:
            row = conn.execute(
                "SELECT * FROM jobs WHERE status='pending'"
                " ORDER BY priority DESC, source_size DESC LIMIT 1"
            ).fetchone()
        if not row:
            _worker_update(worker, "idle")
            return JSONResponse(status_code=204, content=None)
        conn.execute(
            "UPDATE jobs SET status='active', worker=?, updated_at=? WHERE id=?",
            (worker, time.time(), row["id"])
        )
        conn.commit()
    fname = Path(row["source_path"]).name if row["source_path"] else row["id"]
    _worker_update(worker, "encoding", row["id"], fname)
    return dict(row)

@app.post("/jobs/abandon")
def abandon(worker: str = "unknown"):
    with db() as conn:
        conn.execute(
            "UPDATE jobs SET status='pending', worker=NULL, updated_at=? WHERE status='active' AND worker=?",
            (time.time(), worker)
        )
        conn.commit()
    _worker_update(worker, "idle")
    return {"ok": True}

class DeleteOriginalReq(BaseModel):
    rename: bool = False   # rename output to original filename after deleting source

class BulkDeleteReq(BaseModel):
    ids: list
    rename: bool = False

def _do_delete_original(job_id: str, rename: bool) -> dict:
    with db() as conn:
        row = conn.execute(
            "SELECT status, source_path, output_path, source_filename FROM jobs WHERE id=?",
            (job_id,)
        ).fetchone()
    if not row:
        raise HTTPException(404, "job not found")
    if row["status"] != "done":
        raise HTTPException(400, "job not done")

    src = Path(row["source_path"]) if row["source_path"] else None
    out = Path(row["output_path"]) if row["output_path"] else None

    deleted = False
    renamed_to = None

    if src and src.exists():
        src.unlink()
        deleted = True

    if rename and out and out.exists():
        orig_name = row["source_filename"] or (src.name if src else None)
        if orig_name:
            new_path = out.parent / orig_name
            out.rename(new_path)
            with db() as conn:
                conn.execute("UPDATE jobs SET output_path=? WHERE id=?", (str(new_path), job_id))
                conn.commit()
            renamed_to = str(new_path)

    return {"ok": True, "deleted": deleted, "renamed_to": renamed_to}

@app.post("/jobs/bulk-delete-original")
def bulk_delete_original(req: BulkDeleteReq):
    results = []
    for job_id in req.ids:
        try:
            results.append({"id": job_id, **_do_delete_original(job_id, req.rename)})
        except HTTPException as e:
            results.append({"id": job_id, "ok": False, "error": e.detail})
        except Exception as e:
            results.append({"id": job_id, "ok": False, "error": str(e)})
    return {"results": results}

@app.post("/jobs/{job_id}/delete-original")
def delete_original(job_id: str, req: DeleteOriginalReq):
    return _do_delete_original(job_id, req.rename)

class SetPathReq(BaseModel):
    client_path: str

@app.post("/jobs/{job_id}/set-path")
def set_client_path(job_id: str, req: SetPathReq):
    with db() as conn:
        row = conn.execute("SELECT id FROM jobs WHERE id=?", (job_id,)).fetchone()
        if not row:
            raise HTTPException(404, "job not found")
        conn.execute("UPDATE jobs SET client_path=?, updated_at=? WHERE id=?",
                     (req.client_path.strip(), time.time(), job_id))
        conn.commit()
    log.info("Set client_path for %s: %s", job_id, req.client_path.strip())
    return {"ok": True}

@app.post("/jobs/{job_id}/force-encode")
def force_encode(job_id: str):
    """Bump job to top of queue and switch control to run (works from paused state)."""
    with db() as conn:
        row = conn.execute("SELECT status FROM jobs WHERE id=?", (job_id,)).fetchone()
        if not row:
            raise HTTPException(404, "job not found")
        if row["status"] != "pending":
            raise HTTPException(400, "job must be pending")
        conn.execute("UPDATE jobs SET priority=999, updated_at=? WHERE id=?",
                     (time.time(), job_id))
        conn.commit()
    _control["command"] = "run"
    _save_control("run")
    log.info("Force-encode job %s — control set to run", job_id)
    return {"ok": True, "command": "run"}

class ProgressBody(BaseModel):
    worker: str
    phase: str = "encoding"
    percent: float
    fps: float
    speed: str
    frame: int = 0
    bitrate: str = ""
    out_time: str = ""

@app.post("/jobs/{job_id}/progress")
def progress(job_id: str, body: ProgressBody):
    with db() as conn:
        row = conn.execute("SELECT source_path FROM jobs WHERE id=?", (job_id,)).fetchone()
        conn.execute(
            "UPDATE jobs SET percent=?, fps=?, speed=?, updated_at=? WHERE id=?",
            (body.percent, body.fps, body.speed, time.time(), job_id)
        )
        conn.commit()
    fname = Path(row["source_path"]).name if row else ""
    _live[job_id] = {
        "worker": body.worker, "phase": body.phase, "percent": body.percent,
        "fps": body.fps, "speed": body.speed, "frame": body.frame,
        "bitrate": body.bitrate, "out_time": body.out_time,
        "file": fname, "updated_at": time.time(),
    }
    _worker_update(body.worker, body.phase, job_id, fname)
    return {"ok": True}

@app.get("/jobs/live")
def live_progress():
    return _live

class DoneBody(BaseModel):
    worker: str
    output_size: int

def _run_verification(job_id: str, output_path: str, source_duration: float, source_meta_json: str = "{}"):
    def run():
        with db() as conn:
            conn.execute("UPDATE jobs SET verify_status='running', updated_at=? WHERE id=?",
                         (time.time(), job_id))
            conn.commit()
        path = Path(output_path)
        src = {}
        try:
            src = json.loads(source_meta_json or "{}")
        except Exception:
            pass

        checks = []
        overall = "pass"

        if not path.exists():
            _set_verify(job_id, "fail", "output file missing", [], None)
            return

        try:
            out_meta = probe_video(str(path))
        except Exception as e:
            _set_verify(job_id, "fail", f"probe failed: {e}", [], None)
            return

        def chk(name, passed, detail, score=None):
            checks.append({"name": name, "pass": passed, "detail": detail,
                            **({"score": score} if score is not None else {})})

        codec = out_meta.get("video_codec", "")
        chk("codec", codec == "av1", f"got {codec!r}, expected 'av1'")
        if codec != "av1":
            overall = "fail"

        out_dur   = out_meta.get("duration", 0.0)
        dur_diff  = abs(out_dur - source_duration)
        dur_score = max(0.0, 1.0 - dur_diff / 2.0)
        chk("duration", dur_diff <= 2.0,
            f"source={source_duration:.2f}s  output={out_dur:.2f}s  diff={dur_diff:.2f}s",
            round(dur_score, 3))
        if dur_diff > 2.0:
            overall = "fail"

        audio_codecs = out_meta.get("audio_codecs", [])
        audio = audio_codecs[0] if audio_codecs else ""
        chk("audio", audio in ("aac", ""), f"got {audio!r}" if audio else "no audio stream")
        if audio and audio not in ("aac", ""):
            overall = "fail"

        src_w, src_h = src.get("width", 0), src.get("height", 0)
        out_w, out_h = out_meta.get("width", 0), out_meta.get("height", 0)
        res_match = (src_w == out_w and src_h == out_h) or (src_w == 0)
        chk("resolution", res_match,
            f"{out_w}×{out_h}" + (f" (source {src_w}×{src_h})" if not res_match else ""))

        src_frames = src.get("frames", 0)
        out_frames = out_meta.get("frames", 0)
        if src_frames and out_frames:
            frame_diff_pct = abs(out_frames - src_frames) / src_frames
            frame_score    = max(0.0, 1.0 - frame_diff_pct * 10)
            # AV1 containers frequently misreport nb_frames — treat as informational, not fatal
            chk("frames", frame_diff_pct <= 0.05,
                f"source={src_frames}  output={out_frames}  diff={frame_diff_pct*100:.2f}%",
                round(frame_score, 3))

        passed = sum(1 for c in checks if c["pass"])
        _set_verify(job_id, overall, f"{passed}/{len(checks)} checks passed", checks, out_meta)

    threading.Thread(target=run, daemon=True).start()

def _set_verify(job_id: str, status: str, detail: str, checks: list, out_meta):
    with db() as conn:
        conn.execute(
            "UPDATE jobs SET verify_status=?, verify_detail=?, verify_checks=?, output_meta=?, updated_at=? WHERE id=?",
            (status, detail,
             json.dumps(checks) if checks else None,
             json.dumps(out_meta) if out_meta else None,
             time.time(), job_id)
        )
        conn.commit()
    log.info("Job %s verify=%s: %s", job_id, status, detail)

@app.post("/jobs/{job_id}/done")
def done(job_id: str, body: DoneBody):
    with db() as conn:
        row = conn.execute(
            "SELECT source_path, source_duration_secs, output_path, source_meta FROM jobs WHERE id=?",
            (job_id,)
        ).fetchone()
        conn.execute(
            "UPDATE jobs SET status='done', output_size=?, percent=100, updated_at=? WHERE id=?",
            (body.output_size, time.time(), job_id)
        )
        conn.commit()
    _live.pop(job_id, None)
    _worker_update(body.worker, "idle")
    fname    = Path(row["source_path"]).name if row and row["source_path"] else job_id
    size_mb  = body.output_size / 1e6
    log.info("Job %s done (%s, %.1f MB)", job_id, body.worker, size_mb)
    _tg(f"✅ <b>Done</b>  {fname}\n💾 {size_mb:.0f} MB · 🖥 {body.worker}")
    if row and row["output_path"]:
        _run_verification(job_id, row["output_path"], row["source_duration_secs"] or 0,
                          row["source_meta"] or "{}")
    return {"ok": True}

class FailedBody(BaseModel):
    worker: str
    error: str

@app.post("/jobs/{job_id}/failed")
def failed(job_id: str, body: FailedBody):
    with db() as conn:
        conn.execute(
            "UPDATE jobs SET status='failed', error=?, updated_at=? WHERE id=?",
            (body.error, time.time(), job_id)
        )
        conn.commit()
    _live.pop(job_id, None)
    _worker_update(body.worker, "idle")
    with db() as _c:
        _jrow = _c.execute("SELECT source_path FROM jobs WHERE id=?", (job_id,)).fetchone()
    fname = Path(_jrow["source_path"]).name if _jrow and _jrow["source_path"] else job_id
    log.error("Job %s failed (%s): %s", job_id, body.worker, body.error)
    _tg(f"❌ <b>Failed</b>  {fname}\n🖥 {body.worker}\n<code>{body.error[:200]}</code>")
    return {"ok": True}

@app.post("/jobs/backfill-meta")
def backfill_meta():
    def run():
        with db() as conn:
            rows = conn.execute(
                """SELECT id, source_path, output_path, source_meta, output_meta
                   FROM jobs
                   WHERE (source_meta IS NULL OR source_meta='{}' OR output_meta IS NULL)
                   AND source_path IS NOT NULL"""
            ).fetchall()
        log.info("Backfill meta: %d jobs to probe", len(rows))
        for row in rows:
            updates, params = [], []
            if not row["source_meta"] or row["source_meta"] == '{}':
                src = Path(row["source_path"])
                if src.exists():
                    try:
                        m = probe_video(str(src))
                        if m:
                            updates.append("source_meta=?"); params.append(json.dumps(m))
                    except Exception as e:
                        log.warning("Backfill source probe failed %s: %s", src.name, e)
            if not row["output_meta"] and row["output_path"]:
                out = Path(row["output_path"])
                if out.exists():
                    try:
                        m = probe_video(str(out))
                        if m:
                            updates.append("output_meta=?"); params.append(json.dumps(m))
                    except Exception as e:
                        log.warning("Backfill output probe failed %s: %s", out.name, e)
            if updates:
                with db() as conn:
                    conn.execute(
                        f"UPDATE jobs SET {', '.join(updates)}, updated_at=? WHERE id=?",
                        params + [time.time(), row["id"]]
                    )
                    conn.commit()
        log.info("Backfill meta complete")
    threading.Thread(target=run, daemon=True).start()
    return {"ok": True, "message": "backfill started in background"}

@app.post("/jobs/clear-pending")
def clear_pending():
    with db() as conn:
        n = conn.execute("SELECT COUNT(*) FROM jobs WHERE status='pending'").fetchone()[0]
        conn.execute("DELETE FROM jobs WHERE status='pending'")
        conn.commit()
    log.info("Cleared %d pending jobs", n)
    return {"ok": True, "cleared": n}

@app.post("/jobs/clear-failed")
def clear_failed():
    with db() as conn:
        n = conn.execute("SELECT COUNT(*) FROM jobs WHERE status='failed'").fetchone()[0]
        conn.execute("DELETE FROM jobs WHERE status='failed'")
        conn.commit()
    log.info("Cleared %d failed jobs", n)
    return {"ok": True, "cleared": n}

@app.get("/control")
def get_control():
    return _control

@app.post("/control/{cmd}")
def set_control(cmd: str):
    if cmd not in ("run", "drain", "stop"):
        raise HTTPException(400, "cmd must be run | drain | stop")
    _control["command"] = cmd
    _save_control(cmd)
    log.info("Control command set to: %s", cmd)
    return {"ok": True, "command": cmd}

@app.get("/settings")
def get_settings():
    return _all_settings()

@app.post("/settings")
async def update_settings(request: Request):
    data = await request.json()
    with db() as conn:
        for k, v in data.items():
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?,?)", (k, str(v))
            )
        conn.commit()
    return {"ok": True}

@app.get("/jobs")
def list_jobs(
    status: str = None,
    client: str = None,
    sort: str = "smart",
    order: str = "desc",
    page: int = 1,
    per_page: int = 50,
    limit: int = None,
):
    if limit is not None:
        per_page = limit
        page = 1

    filters, params = [], []

    if status:
        statuses = [s.strip() for s in status.split(",") if s.strip()]
        if statuses:
            ph = ",".join("?" * len(statuses))
            filters.append(f"status IN ({ph})")
            params.extend(statuses)

    if client:
        clients = [c.strip() for c in client.split(",") if c.strip()]
        if clients:
            ph = ",".join("?" * len(clients))
            filters.append(f"client_name IN ({ph})")
            params.extend(clients)

    where     = ("WHERE " + " AND ".join(filters)) if filters else ""
    order_dir = "DESC" if order.lower() == "desc" else "ASC"

    sort_clauses = {
        "smart": (
            "CASE status WHEN 'active' THEN 0 WHEN 'pending' THEN 1 WHEN 'done' THEN 2 ELSE 3 END ASC,"
            " CASE WHEN status='pending' THEN source_size END DESC,"
            " updated_at DESC"
        ),
        "size":    f"source_size {order_dir}",
        "name":    f"COALESCE(source_filename, source_path) {order_dir}",
        "updated": f"updated_at {order_dir}",
        "savings": (
            f"CASE WHEN source_size > 0 AND output_size > 0"
            f" THEN (1.0 - CAST(output_size AS REAL)/source_size)"
            f" ELSE -1 END {order_dir}"
        ),
        "height":  f"CAST(json_extract(source_meta, '$.height') AS INTEGER) {order_dir}",
        "status":  f"status {order_dir}, source_size DESC",
    }
    order_clause = sort_clauses.get(sort, sort_clauses["smart"])
    offset = (page - 1) * per_page

    with db() as conn:
        total = conn.execute(f"SELECT COUNT(*) FROM jobs {where}", params).fetchone()[0]
        cnt_rows = conn.execute(
            "SELECT status, COUNT(*) as n FROM jobs GROUP BY status"
        ).fetchall()
        rows = conn.execute(
            f"SELECT * FROM jobs {where} ORDER BY {order_clause} LIMIT ? OFFSET ?",
            params + [per_page, offset]
        ).fetchall()

    counts = {r["status"]: r["n"] for r in cnt_rows}
    def _fixrow(r):
        d = dict(r)
        d["source_filename"] = _fix_mojibake(d.get("source_filename"))
        return d

    return {
        "jobs":     [_fixrow(r) for r in rows],
        "total":    total,
        "page":     page,
        "per_page": per_page,
        "pages":    max(1, (total + per_page - 1) // per_page),
        "counts": {
            "pending": counts.get("pending", 0),
            "active":  counts.get("active",  0),
            "done":    counts.get("done",    0),
            "failed":  counts.get("failed",  0),
        },
    }

@app.post("/scan")
def trigger_scan():
    threading.Thread(target=scan_videos, daemon=True).start()
    return {"ok": True, "message": "scan started"}

@app.get("/jobs/{job_id}/source")
def download_source(job_id: str):
    with db() as conn:
        row = conn.execute("SELECT source_path FROM jobs WHERE id=?", (job_id,)).fetchone()
    if not row:
        raise HTTPException(404, "job not found")
    path = Path(row["source_path"])
    if not path.exists():
        raise HTTPException(404, "source file not found on disk")
    def stream():
        with open(path, "rb") as f:
            while chunk := f.read(1 << 20):
                yield chunk
    return StreamingResponse(stream(), media_type="application/octet-stream",
                             headers={"Content-Length": str(path.stat().st_size),
                                      "X-Filename": quote(path.name)})

@app.put("/jobs/{job_id}/output")
async def upload_output(job_id: str, request: Request):
    with db() as conn:
        row = conn.execute("SELECT output_path FROM jobs WHERE id=?", (job_id,)).fetchone()
    if not row:
        raise HTTPException(404, "job not found")
    out_path = Path(row["output_path"])
    out_path.parent.mkdir(parents=True, exist_ok=True)
    size = 0
    with open(out_path, "wb") as f:
        async for chunk in request.stream():
            f.write(chunk)
            size += len(chunk)
    return {"ok": True, "bytes": size}

@app.get("/jobs/{job_id}")
def get_job(job_id: str):
    with db() as conn:
        row = conn.execute("SELECT * FROM jobs WHERE id=?", (job_id,)).fetchone()
    if not row:
        raise HTTPException(404, "job not found")
    d = dict(row)
    d["source_filename"] = _fix_mojibake(d.get("source_filename"))
    return d

@app.post("/jobs/{job_id}/rescan")
def rescan_job(job_id: str):
    with db() as conn:
        row = conn.execute("SELECT * FROM jobs WHERE id=?", (job_id,)).fetchone()
    if not row:
        raise HTTPException(404, "job not found")
    job = dict(row)

    updated = []
    now = time.time()
    fields: dict = {}

    # ── re-probe source ────────────────────────────────────────────────────────
    src_path = job.get("source_path")
    if src_path and Path(src_path).exists():
        meta = probe_video(src_path)
        if meta:
            fields["source_meta"] = json.dumps(meta)
            fields["source_size"] = Path(src_path).stat().st_size
            fields["source_duration_secs"] = meta.get("duration", job.get("source_duration_secs") or 0)
            if not job.get("source_filename"):
                fields["source_filename"] = Path(src_path).name
            updated.append("source")

    # ── re-probe output + re-verify for done jobs ──────────────────────────────
    out_path = job.get("output_path")
    if job.get("status") == "done" and out_path and Path(out_path).exists():
        out_meta = probe_video(out_path)
        if out_meta:
            # run verification inline (synchronous for rescan)
            src_duration = fields.get("source_duration_secs") or job.get("source_duration_secs") or 0
            src = {}
            try:
                src = json.loads(fields.get("source_meta") or job.get("source_meta") or "{}")
            except Exception:
                pass

            checks = []
            overall = "pass"

            def chk(name, passed, detail, score=None):
                checks.append({"name": name, "pass": passed, "detail": detail,
                                **({"score": score} if score is not None else {})})

            codec = out_meta.get("video_codec", "")
            chk("codec", codec == "av1", f"got {codec!r}, expected 'av1'")
            if codec != "av1":
                overall = "fail"

            out_dur  = out_meta.get("duration", 0.0)
            dur_diff = abs(out_dur - src_duration)
            dur_score = max(0.0, 1.0 - dur_diff / 2.0)
            chk("duration", dur_diff <= 2.0,
                f"source={src_duration:.2f}s  output={out_dur:.2f}s  diff={dur_diff:.2f}s",
                round(dur_score, 3))
            if dur_diff > 2.0:
                overall = "fail"

            audio_codecs = out_meta.get("audio_codecs", [])
            audio = audio_codecs[0] if audio_codecs else ""
            chk("audio", audio in ("aac", ""), f"got {audio!r}" if audio else "no audio stream")
            if audio and audio not in ("aac", ""):
                overall = "fail"

            src_w, src_h = src.get("width", 0), src.get("height", 0)
            out_w, out_h = out_meta.get("width", 0), out_meta.get("height", 0)
            res_match = (src_w == out_w and src_h == out_h) or (src_w == 0)
            chk("resolution", res_match,
                f"{out_w}×{out_h}" + (f" (source {src_w}×{src_h})" if not res_match else ""))

            src_frames = src.get("frames", 0)
            out_frames = out_meta.get("frames", 0)
            if src_frames and out_frames:
                frame_diff_pct = abs(out_frames - src_frames) / src_frames
                frame_score    = max(0.0, 1.0 - frame_diff_pct * 10)
                chk("frames", frame_diff_pct <= 0.05,
                    f"source={src_frames}  output={out_frames}  diff={frame_diff_pct*100:.2f}%",
                    round(frame_score, 3))

            passed = sum(1 for c in checks if c["pass"])
            fields["output_meta"]    = json.dumps(out_meta)
            fields["output_size"]    = Path(out_path).stat().st_size
            fields["verify_status"]  = overall
            fields["verify_detail"]  = f"{passed}/{len(checks)} checks passed"
            fields["verify_checks"]  = json.dumps(checks)
            updated.append("output")
            updated.append("verify")

    if not fields:
        return {"ok": True, "updated": [], "message": "no accessible files found to rescan"}

    fields["updated_at"] = now
    set_clause = ", ".join(f"{k}=?" for k in fields)
    with db() as conn:
        conn.execute(f"UPDATE jobs SET {set_clause} WHERE id=?",
                     list(fields.values()) + [job_id])
        conn.commit()

    log.info("Rescan %s: updated %s", job_id, updated)
    return {"ok": True, "updated": updated,
            "message": f"updated: {', '.join(updated)}" if updated else "nothing to update"}

class BulkRescanReq(BaseModel):
    ids: list

@app.post("/jobs/bulk-rescan")
def bulk_rescan(req: BulkRescanReq):
    results = {}
    for job_id in req.ids:
        with db() as conn:
            row = conn.execute("SELECT * FROM jobs WHERE id=?", (job_id,)).fetchone()
        if not row:
            results[job_id] = {"ok": False, "updated": []}
            continue
        job = dict(row)
        updated = []
        now = time.time()
        fields: dict = {}

        src_path = job.get("source_path")
        if src_path and Path(src_path).exists():
            meta = probe_video(src_path)
            if meta:
                fields["source_meta"] = json.dumps(meta)
                fields["source_size"] = Path(src_path).stat().st_size
                fields["source_duration_secs"] = meta.get("duration", job.get("source_duration_secs") or 0)
                if not job.get("source_filename"):
                    fields["source_filename"] = Path(src_path).name
                updated.append("source")

        out_path = job.get("output_path")
        if job.get("status") == "done" and out_path and Path(out_path).exists():
            out_meta = probe_video(out_path)
            if out_meta:
                src_duration = fields.get("source_duration_secs") or job.get("source_duration_secs") or 0
                src = {}
                try:
                    src = json.loads(fields.get("source_meta") or job.get("source_meta") or "{}")
                except Exception:
                    pass
                checks = []
                overall = "pass"
                def chk(name, passed, detail, score=None):
                    checks.append({"name": name, "pass": passed, "detail": detail,
                                   **({"score": score} if score is not None else {})})
                codec = out_meta.get("video_codec", "")
                chk("codec", codec == "av1", f"got {codec!r}, expected 'av1'")
                if codec != "av1": overall = "fail"
                out_dur = out_meta.get("duration", 0.0)
                dur_diff = abs(out_dur - src_duration)
                chk("duration", dur_diff <= 2.0,
                    f"source={src_duration:.2f}s  output={out_dur:.2f}s  diff={dur_diff:.2f}s",
                    round(max(0.0, 1.0 - dur_diff / 2.0), 3))
                if dur_diff > 2.0: overall = "fail"
                audio = (out_meta.get("audio_codecs") or [""])[0]
                chk("audio", audio in ("aac", ""), f"got {audio!r}" if audio else "no audio stream")
                if audio and audio not in ("aac", ""): overall = "fail"
                src_w, src_h = src.get("width", 0), src.get("height", 0)
                out_w, out_h = out_meta.get("width", 0), out_meta.get("height", 0)
                res_match = (src_w == out_w and src_h == out_h) or (src_w == 0)
                chk("resolution", res_match,
                    f"{out_w}×{out_h}" + (f" (source {src_w}×{src_h})" if not res_match else ""))
                src_frames, out_frames = src.get("frames", 0), out_meta.get("frames", 0)
                if src_frames and out_frames:
                    fdiff = abs(out_frames - src_frames) / src_frames
                    chk("frames", fdiff <= 0.05,
                        f"source={src_frames}  output={out_frames}  diff={fdiff*100:.2f}%",
                        round(max(0.0, 1.0 - fdiff * 10), 3))
                passed = sum(1 for c in checks if c["pass"])
                fields["output_meta"]   = json.dumps(out_meta)
                fields["output_size"]   = Path(out_path).stat().st_size
                fields["verify_status"] = overall
                fields["verify_detail"] = f"{passed}/{len(checks)} checks passed"
                fields["verify_checks"] = json.dumps(checks)
                updated.extend(["output", "verify"])

        if fields:
            fields["updated_at"] = now
            set_clause = ", ".join(f"{k}=?" for k in fields)
            with db() as conn:
                conn.execute(f"UPDATE jobs SET {set_clause} WHERE id=?",
                             list(fields.values()) + [job_id])
                conn.commit()

        results[job_id] = {"ok": True, "updated": updated}
        log.info("Bulk-rescan %s: updated %s", job_id, updated)

    total_updated = sum(len(v["updated"]) > 0 for v in results.values())
    return {"ok": True, "results": results,
            "message": f"{total_updated}/{len(req.ids)} jobs updated"}

UPLOADS_ROOT = Path("/data/.transcode/uploads")

@app.post("/jobs/upload")
async def upload_job(request: Request, x_filename: str = Header(...),
                     x_filepath: str = Header(None)):
    orig_filename = _fix_mojibake(unquote(x_filename))
    client_path   = _fix_mojibake(unquote(x_filepath)) if x_filepath else None
    job_id   = str(uuid.uuid4())
    ext      = Path(orig_filename).suffix or ".mp4"
    upload_dir  = UPLOADS_ROOT / job_id
    upload_dir.mkdir(parents=True, exist_ok=True)
    input_path  = upload_dir / f"input{ext}"
    output_path = upload_dir / f"{Path(orig_filename).stem}_av1.mp4"

    size = 0
    with open(input_path, "wb") as f:
        async for chunk in request.stream():
            f.write(chunk)
            size += len(chunk)

    meta     = probe_video(str(input_path))
    duration = meta.get("duration", 0.0)

    ip = (
        request.headers.get("X-Real-IP")
        or (request.headers.get("X-Forwarded-For", "").split(",")[0])
        or (request.client.host if request.client else "unknown")
    ).strip()

    now = time.time()
    with db() as conn:
        client_name, _ = _get_or_create_client(conn, ip)
        conn.execute("UPDATE clients SET uploads=uploads+1 WHERE ip=?", (ip,))
        conn.execute("""
            INSERT INTO jobs (id, source_path, output_path, source_unc, output_unc,
                source_size, source_duration_secs, status, priority, source_filename,
                source_meta, client_name, created_at, updated_at, client_path)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
        """, (job_id, str(input_path), str(output_path), "", "",
              size, duration, "pending", 10, orig_filename,
              json.dumps(meta), client_name, now, now, client_path))
        conn.commit()
        pos = conn.execute(
            "SELECT COUNT(*) FROM jobs WHERE status='pending' AND priority >= 10 AND id != ?",
            (job_id,)
        ).fetchone()[0]

    log.info("Companion upload [%s/%s]: %s (%.1f GB, %.0fs) → job %s",
             client_name, ip, orig_filename, size/1e9, duration, job_id)
    _tg(f"📤 <b>Upload</b>  {orig_filename}\n👤 {client_name} · 💾 {size/1e9:.2f} GB · queue #{pos+1}")
    return {"job_id": job_id, "priority_position": pos + 1, "client_name": client_name}

@app.get("/jobs/{job_id}/output")
def download_output(job_id: str):
    with db() as conn:
        row = conn.execute(
            "SELECT output_path, source_filename FROM jobs WHERE id=?", (job_id,)
        ).fetchone()
    if not row:
        raise HTTPException(404, "job not found")
    path = Path(row["output_path"])
    if not path.exists():
        raise HTTPException(404, "output not ready")
    original = row["source_filename"] or path.name
    out_name = Path(original).stem + "_av1.mp4"
    def stream():
        with open(path, "rb") as f:
            while chunk := f.read(1 << 20):
                yield chunk
    return StreamingResponse(stream(), media_type="application/octet-stream",
                             headers={"Content-Length": str(path.stat().st_size),
                                      "X-Filename": quote(out_name)})

_COMPANION_BIN = Path(os.getenv("COMPANION_BIN", "/app/enkodu-macos"))

@app.get("/install", response_class=HTMLResponse)
def install_guide():
    return """<!doctype html><html><head><meta charset=utf-8>
    <title>✦ ENKODU — Install Companion</title>
    <style>
      @import url('https://fonts.googleapis.com/css2?family=DM+Mono:wght@400;500&display=swap');
      *{box-sizing:border-box;margin:0;padding:0}
      body{background:#0d0d1a;color:#e0e0e0;font-family:'DM Mono',monospace;font-size:13px;min-height:100vh;padding:40px;max-width:680px;margin:0 auto}
      h1{font-size:22px;font-weight:500;letter-spacing:4px;background:linear-gradient(90deg,#f48fb1,#ce93d8,#80cbc4);-webkit-background-clip:text;-webkit-text-fill-color:transparent;margin-bottom:6px}
      .sub{color:#555;font-size:10px;letter-spacing:2px;margin-bottom:36px}
      h2{color:#ce93d8;font-size:10px;letter-spacing:3px;margin:28px 0 12px}
      p{color:#999;line-height:1.7;margin-bottom:12px}
      .step{background:#13132a;border:1px solid #1e1e3a;border-radius:10px;padding:16px 20px;margin-bottom:10px;display:flex;gap:14px;align-items:flex-start}
      .num{color:#f48fb1;font-size:16px;font-weight:500;min-width:24px;margin-top:1px}
      .step-body{flex:1}
      .step-title{color:#e0e0e0;margin-bottom:6px}
      code{background:#1a1a2e;border:1px solid #2a2a4a;padding:3px 8px;border-radius:5px;color:#80cbc4;font-size:12px;display:inline-block;margin:3px 0}
      .warn{background:#ef9a9a11;border:1px solid #ef9a9a33;border-radius:8px;padding:12px 16px;color:#ef9a9a;font-size:12px;margin:20px 0;line-height:1.6}
      .dl-btn{display:inline-block;background:linear-gradient(90deg,#f48fb133,#ce93d833);border:1px solid #ce93d855;color:#ce93d8;padding:12px 28px;border-radius:10px;text-decoration:none;letter-spacing:2px;font-size:12px;margin:8px 0 24px}
      .dl-btn:hover{background:linear-gradient(90deg,#f48fb155,#ce93d855)}
      a.back{color:#555;font-size:11px;text-decoration:none;letter-spacing:1px}
      a.back:hover{color:#ce93d8}
      hr{border:none;border-top:1px solid #1e1e3a;margin:28px 0}
    </style></head><body>
    <h1>✦ ENKODU</h1>
    <div class="sub">컴패니언 설치 &nbsp;·&nbsp; COMPANION INSTALL GUIDE</div>
    <div class="warn">⚠ This binary is not signed with an Apple Developer certificate.
      macOS Gatekeeper will block it by default. The one-shot command below handles everything.</div>
    <h2>ONE-SHOT INSTALL (recommended)</h2>
    <div class="step" style="display:block;padding:18px 20px;position:relative">
      <p style="color:#ce93d8;margin-bottom:10px">Open Terminal and paste this — downloads, strips quarantine, installs, and launches:</p>
      <div style="position:relative">
        <code id="oneshot" style="display:block;white-space:pre-wrap;line-height:2;font-size:12px;padding-right:80px">curl -fsSL https://enkodu.manwe.qzz.io/download/enkodu -o /tmp/enkodu &amp;&amp; \\
  xattr -d com.apple.quarantine /tmp/enkodu &amp;&amp; \\
  chmod +x /tmp/enkodu &amp;&amp; \\
  sudo mv /tmp/enkodu /usr/local/bin/enkodu &amp;&amp; \\
  enkodu</code>
        <button onclick="copyCmd(this)" style="position:absolute;top:8px;right:8px;background:#1a1a2e;border:1px solid #ce93d855;color:#ce93d8;font-family:'DM Mono',monospace;font-size:10px;letter-spacing:1px;padding:5px 12px;border-radius:6px;cursor:pointer;transition:all .2s">COPY</button>
      </div>
    </div>
    <script>
    function copyCmd(btn) {
      var raw = "curl -fsSL https://enkodu.manwe.qzz.io/download/enkodu -o /tmp/enkodu && \\\n  xattr -d com.apple.quarantine /tmp/enkodu && \\\n  chmod +x /tmp/enkodu && \\\n  sudo mv /tmp/enkodu /usr/local/bin/enkodu && \\\n  enkodu";
      navigator.clipboard.writeText(raw).then(function() {
        btn.textContent = "COPIED"; btn.style.borderColor="#80cbc4"; btn.style.color="#80cbc4";
        setTimeout(function(){btn.textContent="COPY";btn.style.borderColor="#ce93d855";btn.style.color="#ce93d8";},2000);
      });
    }
    </script>
    <h2>MANUAL STEPS</h2>
    <a class="dl-btn" href="/download/enkodu" download="enkodu">⬇ Download enkodu (macOS)</a>
    <div class="step"><div class="num">1</div><div class="step-body"><div class="step-title">Remove quarantine flag</div><code>xattr -d com.apple.quarantine ~/Downloads/enkodu</code></div></div>
    <div class="step"><div class="num">2</div><div class="step-body"><div class="step-title">Make executable and move to PATH</div><code>chmod +x ~/Downloads/enkodu</code><br><code>sudo mv ~/Downloads/enkodu /usr/local/bin/enkodu</code></div></div>
    <div class="step"><div class="num">3</div><div class="step-body"><div class="step-title">Launch it</div><code>enkodu</code></div></div>
    <hr>
    <a class="back" href="/">← back to dashboard</a>
    </body></html>"""

@app.get("/download/enkodu")
def download_companion():
    if not _COMPANION_BIN.exists():
        raise HTTPException(404, "companion binary not found — rebuild and mount")
    def stream():
        with open(_COMPANION_BIN, "rb") as f:
            while chunk := f.read(1 << 20):
                yield chunk
    return StreamingResponse(stream(), media_type="application/octet-stream",
                             headers={"Content-Length": str(_COMPANION_BIN.stat().st_size),
                                      "Content-Disposition": "attachment; filename=enkodu"})

@app.post("/jobs/{job_id}/requeue")
def requeue(job_id: str):
    with db() as conn:
        conn.execute(
            "UPDATE jobs SET status='pending', worker=NULL, error=NULL, percent=0, updated_at=? WHERE id=?",
            (time.time(), job_id)
        )
        conn.commit()
    return {"ok": True}

@app.get("/stats")
def get_stats():
    with db() as conn:
        cnt_rows = conn.execute(
            "SELECT status, COUNT(*) as n FROM jobs GROUP BY status"
        ).fetchall()
        counts = {r["status"]: r["n"] for r in cnt_rows}

        done_row = conn.execute("""
            SELECT COUNT(*) as n,
                   COALESCE(SUM(source_size), 0) as src_bytes,
                   COALESCE(SUM(CASE WHEN output_size IS NOT NULL THEN output_size ELSE 0 END), 0) as out_bytes,
                   COALESCE(SUM(source_duration_secs), 0) as dur_secs
            FROM jobs WHERE status='done'
        """).fetchone()

        savings_rows = conn.execute("""
            SELECT (1.0 - CAST(output_size AS REAL)/source_size)*100 as pct
            FROM jobs WHERE status='done' AND source_size > 0 AND output_size > 0
            ORDER BY pct
        """).fetchall()

        codec_rows = conn.execute("""
            SELECT COALESCE(json_extract(source_meta,'$.video_codec'),'unknown') as codec,
                   COUNT(*) as n
            FROM jobs WHERE status='pending' AND source_meta IS NOT NULL
            GROUP BY codec ORDER BY n DESC LIMIT 10
        """).fetchall()

        res_rows = conn.execute("""
            SELECT CASE
                WHEN CAST(json_extract(source_meta,'$.height') AS INTEGER) >= 2160 THEN '4K'
                WHEN CAST(json_extract(source_meta,'$.height') AS INTEGER) >= 1080 THEN '1080p'
                WHEN CAST(json_extract(source_meta,'$.height') AS INTEGER) >= 720  THEN '720p'
                WHEN CAST(json_extract(source_meta,'$.height') AS INTEGER) >= 480  THEN '480p'
                WHEN CAST(json_extract(source_meta,'$.height') AS INTEGER) > 0     THEN '<480p'
                ELSE 'unknown' END as bucket,
                COUNT(*) as n,
                COALESCE(SUM(source_size),0) as bytes
            FROM jobs WHERE status='pending'
            GROUP BY bucket ORDER BY n DESC
        """).fetchall()

        pend_size = conn.execute(
            "SELECT COALESCE(SUM(source_size),0) FROM jobs WHERE status='pending'"
        ).fetchone()[0]

        client_rows = conn.execute("""
            SELECT client_name,
                   SUM(CASE WHEN status='done'    THEN 1 ELSE 0 END) as done,
                   SUM(CASE WHEN status='pending' THEN 1 ELSE 0 END) as pending,
                   SUM(CASE WHEN status='active'  THEN 1 ELSE 0 END) as active,
                   SUM(CASE WHEN status='failed'  THEN 1 ELSE 0 END) as failed,
                   COALESCE(SUM(CASE WHEN status='done' THEN source_size ELSE 0 END),0) as done_src,
                   COALESCE(SUM(CASE WHEN status='done' AND output_size IS NOT NULL THEN output_size ELSE 0 END),0) as done_out,
                   COALESCE(SUM(CASE WHEN status='pending' THEN source_size ELSE 0 END),0) as pend_src
            FROM jobs GROUP BY client_name ORDER BY done DESC, pending DESC
        """).fetchall()

    pcts = [r["pct"] for r in savings_rows]
    savings_stats = {}
    if pcts:
        savings_stats = {
            "min": round(min(pcts), 1),
            "max": round(max(pcts), 1),
            "avg": round(sum(pcts) / len(pcts), 1),
            "median": round(pcts[len(pcts) // 2], 1),
        }

    return {
        "counts": counts,
        "done": {
            "n": done_row["n"],
            "src_bytes": done_row["src_bytes"],
            "out_bytes": done_row["out_bytes"],
            "dur_secs":  done_row["dur_secs"],
        },
        "savings": savings_stats,
        "pending": {
            "total_bytes": pend_size,
            "codecs":      [dict(r) for r in codec_rows],
            "resolutions": [dict(r) for r in res_rows],
        },
        "clients": [dict(r) for r in client_rows],
    }


@app.get("/", response_class=HTMLResponse)
def dashboard():
    stall_to = STALL_TIMEOUT
    nas_data_root = _get_setting("nas_data_root", "/mnt/pool1/pool1_data/home/yulia")
    return f"""<!doctype html>
<html><head><meta charset=utf-8><title>✦ ENKODU</title>
<style>
@import url('https://fonts.googleapis.com/css2?family=DM+Mono:wght@400;500&display=swap');
*{{box-sizing:border-box;margin:0;padding:0}}
body{{background:#0d0d1a;color:#e0e0e0;font-family:'DM Mono',monospace;font-size:13px;min-height:100vh;padding:28px 32px}}
.header{{display:flex;align-items:center;gap:14px;margin-bottom:20px}}
.logo{{font-size:24px;font-weight:500;letter-spacing:4px;background:linear-gradient(90deg,#f48fb1,#ce93d8,#80cbc4);-webkit-background-clip:text;-webkit-text-fill-color:transparent;cursor:pointer}}
.sub{{color:#444;font-size:11px;letter-spacing:2px}}
.header-right{{margin-left:auto;display:flex;align-items:center;gap:12px}}
.tab-bar{{display:flex;gap:2px;background:#0a0a18;border:1px solid #1e1e3a;border-radius:10px;padding:3px}}
.tab{{background:none;border:none;color:#555;font-family:inherit;font-size:10px;letter-spacing:2px;padding:6px 16px;border-radius:8px;cursor:pointer;transition:all .2s}}
.tab.active{{background:#1e1e3a;color:#e0e0e0}}
.tab:hover:not(.active){{color:#999}}
.install-link{{font-size:10px;letter-spacing:2px;color:#ce93d8;border:1px solid #ce93d844;padding:5px 12px;border-radius:8px;text-decoration:none;background:#ce93d811}}
.install-link:hover{{background:#ce93d822}}
.controls{{display:flex;gap:8px;margin-bottom:22px;flex-wrap:wrap;align-items:center}}
.btn{{border:none;border-radius:8px;padding:7px 14px;font-family:inherit;font-size:10px;letter-spacing:1.5px;cursor:pointer;transition:all .2s;opacity:.55}}
.btn:hover{{opacity:.85}}
.btn-run{{background:#80cbc433;color:#80cbc4;border:1px solid #80cbc455}}
.btn-drain{{background:#ce93d833;color:#ce93d8;border:1px solid #ce93d855}}
.btn-stop{{background:#ef9a9a33;color:#ef9a9a;border:1px solid #ef9a9a55}}
.btn-neutral{{background:#b39ddb22;color:#b39ddb;border:1px solid #b39ddb44;opacity:1}}
.btn-run.active{{background:#80cbc4;color:#0d0d1a;opacity:1;box-shadow:0 0 12px #80cbc466}}
.btn-drain.active{{background:#ce93d8;color:#0d0d1a;opacity:1;box-shadow:0 0 12px #ce93d866}}
.btn-stop.active{{background:#ef9a9a;color:#0d0d1a;opacity:1;box-shadow:0 0 12px #ef9a9a66}}
.cmd-badge{{font-size:10px;letter-spacing:2px;color:#444;margin-left:4px}}
.live-card{{background:#13132a;border:1px solid #f48fb133;border-radius:12px;padding:16px 20px;margin-bottom:20px;display:none}}
.live-card.active{{display:block}}
.live-header{{display:flex;align-items:center;gap:12px;margin-bottom:8px}}
.live-title{{color:#f48fb1;font-size:10px;letter-spacing:3px}}
.live-phase{{font-size:10px;letter-spacing:2px;padding:2px 10px;border-radius:20px}}
.phase-encoding{{background:#f48fb122;color:#f48fb1;border:1px solid #f48fb144}}
.phase-uploading{{background:#80cbc422;color:#80cbc4;border:1px solid #80cbc444}}
.phase-verifying{{background:#ce93d822;color:#ce93d8;border:1px solid #ce93d844}}
.live-file{{color:#e0e0e0;font-size:13px;margin-bottom:10px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}}
.live-bar-bg{{background:#1a1a2e;border-radius:20px;height:6px;margin-bottom:8px;overflow:hidden}}
.live-bar{{background:linear-gradient(90deg,#f48fb1,#ce93d8,#80cbc4);height:6px;border-radius:20px;transition:width .5s}}
@keyframes slide{{0%{{transform:translateX(-100%)}}100%{{transform:translateX(350%)}}}}
.live-bar-ind{{background:linear-gradient(90deg,transparent,#80cbc4,transparent);height:6px;border-radius:20px;width:40%;animation:slide 1.5s ease-in-out infinite}}
.live-stats{{display:flex;gap:20px;flex-wrap:wrap}}
.live-stat{{color:#555;font-size:11px}}.live-stat span{{color:#ce93d8}}
.stats-row{{display:flex;gap:12px;margin-bottom:20px;flex-wrap:wrap;align-items:stretch}}
.cards{{display:flex;gap:10px;flex-wrap:wrap}}
.card{{background:#13132a;border:1px solid #1e1e3a;border-radius:12px;padding:14px 20px;min-width:100px;cursor:pointer;transition:border-color .2s}}
.card:hover{{border-color:#2a2a4a}}
.card.active-filter{{border-color:#ce93d8;box-shadow:0 0 8px #ce93d822}}
.card-num{{font-size:26px;font-weight:500}}
.card-label{{color:#555;font-size:10px;letter-spacing:2px;margin-top:2px}}
.progress-outer{{background:#13132a;border:1px solid #1e1e3a;border-radius:12px;padding:14px 20px;flex:1;min-width:200px}}
.progress-label{{color:#444;font-size:10px;letter-spacing:2px;margin-bottom:8px}}
.progress-bar-bg{{background:#1a1a2e;border-radius:20px;height:8px}}
.progress-bar{{background:linear-gradient(90deg,#f48fb1,#ce93d8,#80cbc4);height:8px;border-radius:20px;transition:width .5s}}
.progress-pct{{color:#ce93d8;font-size:11px;margin-top:6px}}
.workers-section{{margin-bottom:20px;display:none}}
.workers-label{{color:#444;font-size:10px;letter-spacing:2px;margin-bottom:8px}}
.filter-bar{{display:flex;gap:16px;align-items:flex-start;margin-bottom:14px;flex-wrap:wrap}}
.status-pills{{display:flex;gap:6px;flex-wrap:wrap}}
.spill{{background:none;border:1px solid #2a2a3a;color:#555;font-family:inherit;font-size:10px;letter-spacing:1.5px;padding:5px 12px;border-radius:20px;cursor:pointer;transition:all .2s}}
.spill:hover{{border-color:#555;color:#999}}
.spill.active{{border-color:#ce93d8;color:#ce93d8;background:#ce93d811}}
.client-chips{{display:flex;gap:6px;flex-wrap:wrap}}
.chip{{border-radius:20px;padding:4px 12px;font-size:10px;letter-spacing:1px;cursor:pointer;transition:all .2s;border-width:1px;border-style:solid}}
.chip.active{{box-shadow:0 0 8px rgba(255,255,255,.15)}}
table{{border-collapse:collapse;width:100%;background:#13132a;border-radius:12px;overflow:hidden;border:1px solid #1e1e3a}}
th{{color:#444;font-size:10px;letter-spacing:2px;padding:9px 12px;border-bottom:1px solid #1e1e3a;text-align:left;font-weight:400;white-space:nowrap}}
th.sortable{{cursor:pointer;user-select:none}}
th.sortable:hover{{color:#999}}
th.sort-active{{color:#ce93d8}}
td{{padding:8px 12px;border-bottom:1px solid #111827;vertical-align:middle}}
tr:last-child td{{border-bottom:none}}
tr.job-row{{cursor:pointer}}
tr.job-row:hover td{{background:#16162e}}
tr.detail-row td{{background:#0f0f22;padding:0}}
.detail-panel{{padding:14px 18px;display:grid;grid-template-columns:1fr 1fr;gap:14px;font-size:11px}}
.detail-section{{background:#13132a;border:1px solid #1e1e3a;border-radius:8px;padding:12px 14px}}
.detail-title{{color:#555;font-size:9px;letter-spacing:2px;margin-bottom:10px}}
.detail-grid{{display:grid;grid-template-columns:max-content 1fr;gap:3px 12px}}
.dk{{color:#555}}.dv{{color:#e0e0e0}}
.checks{{grid-column:1/-1}}
.check-row{{display:flex;align-items:center;gap:8px;padding:3px 0;border-bottom:1px solid #1a1a2e}}
.check-row:last-child{{border-bottom:none}}
.check-pass{{color:#80cbc4}}.check-fail{{color:#ef9a9a}}
.check-name{{color:#b39ddb;min-width:90px}}.check-detail{{color:#777;flex:1}}
.check-score{{color:#ce93d8;font-size:10px}}
.pagination{{display:flex;gap:6px;margin-top:14px;align-items:center;flex-wrap:wrap}}
.ppage{{background:none;border:1px solid #1e1e3a;color:#555;font-family:inherit;font-size:10px;letter-spacing:1px;padding:5px 11px;border-radius:8px;cursor:pointer;transition:all .2s}}
.ppage:hover{{border-color:#444;color:#999}}
.ppage.active{{border-color:#ce93d8;color:#ce93d8;background:#ce93d811}}
.ppage:disabled{{opacity:.3;cursor:default}}
.page-info{{color:#444;font-size:10px;letter-spacing:1px;margin:0 6px}}
/* settings tab */
#tab-settings{{max-width:640px}}
.settings-card{{background:#13132a;border:1px solid #1e1e3a;border-radius:12px;padding:24px 28px;margin-bottom:20px}}
.settings-title{{color:#ce93d8;font-size:10px;letter-spacing:3px;margin-bottom:6px}}
.settings-desc{{color:#555;font-size:11px;margin-bottom:20px}}
.settings-grid{{display:grid;grid-template-columns:180px 1fr;gap:14px 20px;align-items:center;margin-bottom:24px}}
.settings-label{{color:#999;font-size:11px;letter-spacing:1px}}
.settings-hint{{color:#444;font-size:10px;margin-top:3px}}
.sinput{{background:#0d0d1a;border:1px solid #2a2a3a;color:#e0e0e0;font-family:inherit;font-size:12px;padding:7px 12px;border-radius:8px;width:120px;transition:border-color .2s}}
.sinput:focus{{outline:none;border-color:#ce93d8}}
.sselect{{background:#0d0d1a;border:1px solid #2a2a3a;color:#e0e0e0;font-family:inherit;font-size:12px;padding:7px 12px;border-radius:8px;cursor:pointer}}
.sselect:focus{{outline:none;border-color:#ce93d8}}
.stoggle{{display:flex;align-items:center;gap:10px}}
.toggle-track{{width:40px;height:22px;background:#1a1a2e;border:1px solid #2a2a3a;border-radius:20px;cursor:pointer;position:relative;transition:background .2s}}
.toggle-track.on{{background:#ce93d844;border-color:#ce93d8}}
.toggle-thumb{{width:16px;height:16px;background:#444;border-radius:50%;position:absolute;top:2px;left:2px;transition:all .2s}}
.toggle-track.on .toggle-thumb{{left:20px;background:#ce93d8}}
.btn-save{{background:linear-gradient(90deg,#f48fb133,#ce93d833);border:1px solid #ce93d855;color:#ce93d8;font-family:inherit;font-size:11px;letter-spacing:2px;padding:10px 24px;border-radius:10px;cursor:pointer;transition:all .2s}}
.btn-save:hover{{background:linear-gradient(90deg,#f48fb155,#ce93d855)}}
.save-ok{{color:#80cbc4;font-size:11px;margin-left:12px;opacity:0;transition:opacity .3s}}
a{{color:#f48fb1;text-decoration:none}}
/* delete + force + rescan + select */
.btn-del-orig{{background:#ef9a9a22;border:1px solid #ef9a9a55;color:#ef9a9a;font-family:inherit;font-size:10px;letter-spacing:1.5px;padding:6px 16px;border-radius:8px;cursor:pointer;transition:all .2s}}
.btn-del-orig:hover{{background:#ef9a9a44}}
.btn-del-orig:disabled{{opacity:.3;cursor:default}}
.btn-force-enc{{background:#80cbc422;border:1px solid #80cbc455;color:#80cbc4;font-family:inherit;font-size:10px;letter-spacing:1.5px;padding:6px 16px;border-radius:8px;cursor:pointer;transition:all .2s}}
.btn-force-enc:hover{{background:#80cbc444}}
.btn-rescan{{background:#b39ddb22;border:1px solid #b39ddb55;color:#b39ddb;font-family:inherit;font-size:10px;letter-spacing:1.5px;padding:6px 16px;border-radius:8px;cursor:pointer;transition:all .2s}}
.btn-rescan:hover{{background:#b39ddb44}}
.btn-rescan:disabled{{opacity:.3;cursor:default}}
.job-row.selected td{{background:#ce93d811!important;box-shadow:inset 0 0 0 1px #ce93d833}}
.bulk-bar{{position:fixed;bottom:28px;left:50%;transform:translateX(-50%);background:#1e1e3a;border:1px solid #2a2a4a;border-radius:14px;padding:12px 20px;display:flex;align-items:center;gap:14px;box-shadow:0 4px 32px #000c;z-index:200;white-space:nowrap}}
.bulk-bar-count{{color:#ce93d8;font-size:12px;letter-spacing:1px}}
.bulk-bar-label{{color:#777;font-size:11px;display:flex;align-items:center;gap:6px;cursor:pointer}}
.path-box{{font-size:11px;word-break:break-all;line-height:1.6;background:#0d0d1a;border:1px solid #1e1e3a;border-radius:8px;padding:10px 14px;margin-top:10px;display:grid;grid-template-columns:60px 1fr;gap:4px 10px}}
.weight-row{{display:grid;grid-template-columns:140px 1fr 60px 80px;gap:10px;align-items:center;margin-bottom:10px}}
.weight-row-name{{color:#e0e0e0;font-size:12px}}
.weight-slider{{-webkit-appearance:none;appearance:none;height:6px;border-radius:3px;background:#1a1a2e;outline:none;cursor:pointer}}
.weight-slider::-webkit-slider-thumb{{-webkit-appearance:none;width:16px;height:16px;border-radius:50%;background:#ce93d8;cursor:pointer}}
.weight-val{{color:#ce93d8;font-size:12px;text-align:right}}
.weight-pct{{color:#555;font-size:10px;letter-spacing:1px}}
/* report tab */
.report-grid{{display:grid;grid-template-columns:repeat(4,1fr);gap:12px;margin-bottom:24px}}
.rcard{{background:#13132a;border:1px solid #1e1e3a;border-radius:12px;padding:18px 20px}}
.rcard-val{{font-size:28px;font-weight:500;line-height:1}}
.rcard-unit{{font-size:11px;color:#555;margin-top:3px;letter-spacing:1px}}
.rcard-label{{font-size:10px;letter-spacing:2px;color:#444;margin-bottom:10px}}
.report-row{{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-bottom:24px}}
.report-section{{background:#13132a;border:1px solid #1e1e3a;border-radius:12px;padding:18px 20px}}
.report-section.full{{grid-column:1/-1}}
.rsec-title{{color:#444;font-size:10px;letter-spacing:2px;margin-bottom:14px}}
.bar-row{{display:flex;align-items:center;gap:10px;margin-bottom:8px}}
.bar-label{{color:#999;font-size:11px;min-width:70px;text-align:right}}
.bar-track{{flex:1;background:#1a1a2e;border-radius:20px;height:8px;overflow:hidden}}
.bar-fill{{height:8px;border-radius:20px;transition:width .6s}}
.bar-count{{color:#555;font-size:10px;min-width:40px}}
.bar-bytes{{color:#444;font-size:9px;min-width:60px}}
.savings-range{{display:flex;align-items:center;gap:10px;margin-top:10px}}
.sr-pill{{background:#1a1a2e;border:1px solid #2a2a3a;border-radius:8px;padding:8px 14px;text-align:center;flex:1}}
.sr-pill-val{{font-size:18px;font-weight:500;color:#ce93d8}}
.sr-pill-lbl{{font-size:9px;color:#444;letter-spacing:1px;margin-top:2px}}
.savings-bar-wrap{{position:relative;height:12px;background:#1a1a2e;border-radius:20px;margin:12px 0}}
.savings-bar-fill{{position:absolute;height:12px;border-radius:20px;background:linear-gradient(90deg,#80cbc4,#ce93d8,#f48fb1)}}
.savings-bar-marker{{position:absolute;top:-4px;height:20px;width:2px;background:#fff3}}
.savings-bar-avg{{position:absolute;top:-4px;height:20px;width:2px;background:#ce93d8}}
.rclient-table{{width:100%;border-collapse:collapse}}
.rclient-table th{{color:#444;font-size:9px;letter-spacing:2px;padding:0 8px 10px;text-align:right;font-weight:400}}
.rclient-table th:first-child{{text-align:left}}
.rclient-table td{{padding:6px 8px;border-top:1px solid #1a1a2e;font-size:11px;text-align:right}}
.rclient-table td:first-child{{text-align:left;color:#e0e0e0}}
.rclient-table tr:first-child td{{border-top:none}}
@media(max-width:800px){{.report-grid{{grid-template-columns:repeat(2,1fr)}}.report-row{{grid-template-columns:1fr}}}}
</style>
</head>
<body>
<div class="header">
  <div class="logo" onclick="switchTab('queue')" title="back to queue">✦ ENKODU</div>
  <div class="sub">인코딩 서버 · AV1 TRANSCODER</div>
  <div class="header-right">
    <div class="tab-bar">
      <button class="tab active" id="tab-btn-queue" onclick="switchTab('queue')">QUEUE</button>
      <button class="tab" id="tab-btn-report" onclick="switchTab('report')">REPORT</button>
      <button class="tab" id="tab-btn-settings" onclick="switchTab('settings')">SETTINGS</button>
    </div>
    <a class="install-link" href="/install">⬇ ENKODU.APP</a>
  </div>
</div>

<!-- ── QUEUE TAB ── -->
<div id="tab-queue">
  <div class="controls">
    <button class="btn btn-run"     id="btn-run"   onclick="ctrl('run')">▶ 재개 RESUME</button>
    <button class="btn btn-drain"   id="btn-drain" onclick="ctrl('drain')">⏸ 현재 후 중지 DRAIN</button>
    <button class="btn btn-stop"    id="btn-stop"  onclick="ctrl('stop')">⏹ 지금 중지 STOP NOW</button>
    <button class="btn btn-neutral" onclick="post('/scan')">⟳ 스캔 RESCAN</button>
    <button class="btn btn-neutral" onclick="postAndRefresh('/jobs/clear-failed')">✕ 실패 삭제 CLEAR FAILED</button>
    <button class="btn btn-neutral" onclick="postAndRefresh('/jobs/clear-pending')">✕ 대기 삭제 CLEAR PENDING</button>
    <button class="btn btn-neutral" onclick="post('/jobs/backfill-meta')">⟳ BACKFILL META</button>
    <button class="btn btn-neutral" id="btn-select" onclick="toggleSelectMode()">⊙ SELECT</button>
    <button class="btn btn-neutral" id="btn-nas-drain" onclick="toggleNasDrain()">⏸ NAS SCAN</button>
    <span class="cmd-badge">명령 COMMAND: <span id="cmd-badge">—</span></span>
  </div>

  <div class="bulk-bar" id="bulk-bar" style="display:none">
    <span class="bulk-bar-count" id="bulk-count">0 selected</span>
    <label class="bulk-bar-label">
      <input type="checkbox" id="bulk-rename" style="accent-color:#ce93d8"> rename to original name
    </label>
    <button class="btn-del-orig" onclick="bulkDeleteOriginal()">✕ DELETE ORIGINAL</button>
    <button class="btn-rescan" id="bulk-rescan-btn" onclick="bulkRescan()">⟳ RESCAN</button>
    <button class="btn btn-neutral" style="opacity:1;padding:5px 12px" onclick="exitSelectMode()">✕ cancel</button>
  </div>

  <div class="live-card" id="live-card">
    <div class="live-header">
      <div class="live-title">▶ 작업 중 &nbsp; NOW WORKING</div>
      <div class="live-phase phase-encoding" id="live-phase">ENCODING</div>
    </div>
    <div class="live-file" id="live-file"></div>
    <div class="live-bar-bg" id="live-bar-bg"><div class="live-bar" id="live-bar" style="width:0%"></div></div>
    <div class="live-stats">
      <div class="live-stat">진행률 <span id="live-pct">0%</span></div>
      <div class="live-stat">FPS <span id="live-fps">—</span></div>
      <div class="live-stat">속도 SPEED <span id="live-speed">—</span></div>
      <div class="live-stat">프레임 FRAME <span id="live-frame">—</span></div>
      <div class="live-stat">비트레이트 <span id="live-bitrate">—</span></div>
      <div class="live-stat">시간 TIME <span id="live-time">—</span></div>
      <div class="live-stat">워커 WORKER <span id="live-worker">—</span></div>
    </div>
  </div>

  <div class="stats-row">
    <div class="cards">
      <div class="card" id="card-all"     onclick="setStatusFilter([])"           title="Show all"><div class="card-num" id="cnt-total" style="color:#e0e0e0">—</div><div class="card-label">전체 ALL</div></div>
      <div class="card" id="card-pending" onclick="setStatusFilter(['pending'])"  title="Pending only"><div class="card-num" id="cnt-pending" style="color:#b39ddb">—</div><div class="card-label">대기중 PENDING</div></div>
      <div class="card" id="card-active"  onclick="setStatusFilter(['active'])"   title="Active only"><div class="card-num" id="cnt-active"  style="color:#f48fb1">—</div><div class="card-label">인코딩 ACTIVE</div></div>
      <div class="card" id="card-done"    onclick="setStatusFilter(['done'])"     title="Done only"><div class="card-num" id="cnt-done"    style="color:#80cbc4">—</div><div class="card-label">완료 DONE</div></div>
      <div class="card" id="card-failed"  onclick="setStatusFilter(['failed'])"   title="Failed only"><div class="card-num" id="cnt-failed"  style="color:#ef9a9a">—</div><div class="card-label">실패 FAILED</div></div>
    </div>
    <div class="progress-outer">
      <div class="progress-label">전체 진행률 &nbsp; OVERALL PROGRESS</div>
      <div class="progress-bar-bg"><div class="progress-bar" id="prog-bar" style="width:0%"></div></div>
      <div class="progress-pct" id="prog-pct">0%</div>
    </div>
  </div>

  <div class="workers-section" id="workers-section">
    <div class="workers-label">워커 WORKERS</div>
    <div id="workers-badges" style="display:flex;gap:8px;flex-wrap:wrap"></div>
  </div>

  <div class="filter-bar">
    <div class="status-pills" id="status-pills">
      <button class="spill active" data-status="" onclick="setStatusFilter([])">ALL</button>
      <button class="spill" data-status="pending" onclick="toggleStatusPill('pending')">PENDING</button>
      <button class="spill" data-status="active"  onclick="toggleStatusPill('active')">ACTIVE</button>
      <button class="spill" data-status="done"    onclick="toggleStatusPill('done')">DONE</button>
      <button class="spill" data-status="failed"  onclick="toggleStatusPill('failed')">FAILED</button>
    </div>
    <div class="client-chips" id="client-chips"></div>
  </div>

  <table>
    <thead><tr>
      <th style="width:90px">상태 STATUS</th>
      <th class="sortable" onclick="setSort('name')" id="th-name">FILE</th>
      <th class="sortable" onclick="setSort('height')" id="th-height" style="width:80px">RES</th>
      <th class="sortable" onclick="setSort('size')" id="th-size" style="width:90px">SIZE</th>
      <th class="sortable" onclick="setSort('savings')" id="th-savings" style="width:100px">SAVINGS</th>
      <th style="width:140px">PROGRESS</th>
      <th style="width:110px">CLIENT</th>
      <th style="width:110px">WORKER</th>
    </tr></thead>
    <tbody id="jobs-tbody"><tr><td colspan="8" style="color:#444;text-align:center;padding:32px">loading…</td></tr></tbody>
  </table>
  <div class="pagination" id="pagination"></div>
</div>

<!-- ── SETTINGS TAB ── -->
<div id="tab-settings" style="display:none">
  <div class="settings-card">
    <div class="settings-title">QUEUE STRATEGY</div>
    <div class="settings-desc">Filters applied during scan. Files that don't match are silently skipped. Existing jobs are not affected — re-scan after saving.</div>
    <div class="settings-grid">
      <div>
        <div class="settings-label">Min file size</div>
        <div class="settings-hint">0 = no limit</div>
      </div>
      <div style="display:flex;align-items:center;gap:8px">
        <input class="sinput" type="number" id="s-min-size" min="0" step="100" value="0">
        <span style="color:#555;font-size:11px">MB</span>
      </div>

      <div>
        <div class="settings-label">Min resolution</div>
        <div class="settings-hint">skip files below this height</div>
      </div>
      <select class="sselect" id="s-min-height">
        <option value="0">Any resolution</option>
        <option value="480">480p+</option>
        <option value="720">720p+</option>
        <option value="1080">1080p+</option>
        <option value="2160">4K (2160p+)</option>
      </select>

      <div>
        <div class="settings-label">Min bitrate</div>
        <div class="settings-hint">0 = no limit (kbps)</div>
      </div>
      <div style="display:flex;align-items:center;gap:8px">
        <input class="sinput" type="number" id="s-min-bitrate" min="0" step="500" value="0">
        <span style="color:#555;font-size:11px">kbps</span>
      </div>

      <div class="settings-label">Skip HEVC / H.265 files</div>
      <div class="stoggle">
        <div class="toggle-track" id="tog-hevc" onclick="toggleSetting('tog-hevc')"><div class="toggle-thumb"></div></div>
        <span id="tog-hevc-label" style="color:#555;font-size:11px">OFF</span>
      </div>

      <div class="settings-label">Skip AV1 files</div>
      <div class="stoggle">
        <div class="toggle-track" id="tog-av1" onclick="toggleSetting('tog-av1')"><div class="toggle-thumb"></div></div>
        <span id="tog-av1-label" style="color:#555;font-size:11px">OFF</span>
      </div>
    </div>
    <button class="btn-save" onclick="saveSettings()">SAVE SETTINGS</button>
    <span class="save-ok" id="save-ok">✓ saved</span>
  </div>

  <div class="settings-card">
    <div class="settings-title">CLIENT PRIORITY</div>
    <div class="settings-desc">Weighted fair queuing — each job dispatch picks the client most behind its share. Weights are relative; they don't need to sum to 100.</div>
    <div id="client-weights-list" style="margin-bottom:20px"></div>
    <button class="btn-save" onclick="saveClientWeights()">SAVE WEIGHTS</button>
    <span class="save-ok" id="save-weights-ok">✓ saved</span>
  </div>

  <div class="settings-card">
    <div class="settings-title">COMPANION QUEUE</div>
    <div class="settings-desc">Files queued in companion apps waiting to be uploaded. Companion must call <code style="color:#80cbc4;font-size:10px">POST /clients/queue-manifest</code> to register its local queue.</div>
    <div id="companion-queue-list" style="color:#444;font-size:11px">loading…</div>
  </div>
</div>

<!-- ── REPORT TAB ── -->
<div id="tab-report" style="display:none">
  <div class="report-grid" id="rep-cards">
    <div class="rcard"><div class="rcard-label">공간 절약 SPACE SAVED</div><div class="rcard-val" id="rep-saved" style="color:#80cbc4">—</div><div class="rcard-unit" id="rep-saved-pct"></div></div>
    <div class="rcard"><div class="rcard-label">완료 DONE</div><div class="rcard-val" id="rep-done" style="color:#80cbc4">—</div><div class="rcard-unit" id="rep-done-of"></div></div>
    <div class="rcard"><div class="rcard-label">평균 절약 AVG SAVINGS</div><div class="rcard-val" id="rep-avg" style="color:#ce93d8">—</div><div class="rcard-unit" id="rep-med"></div></div>
    <div class="rcard"><div class="rcard-label">트랜스코딩 시간 TRANSCODED</div><div class="rcard-val" id="rep-hours" style="color:#b39ddb">—</div><div class="rcard-unit">hours of content</div></div>
  </div>

  <div class="report-row">
    <div class="report-section">
      <div class="rsec-title">해상도별 백로그 BACKLOG BY RESOLUTION</div>
      <div id="rep-res-bars"></div>
    </div>
    <div class="report-section">
      <div class="rsec-title">코덱별 백로그 BACKLOG BY CODEC</div>
      <div id="rep-codec-bars"></div>
    </div>
  </div>

  <div class="report-row">
    <div class="report-section">
      <div class="rsec-title">절약 분포 SAVINGS DISTRIBUTION</div>
      <div id="rep-savings-dist"></div>
    </div>
    <div class="report-section">
      <div class="rsec-title">클라이언트별 BY CLIENT</div>
      <table class="rclient-table">
        <thead><tr>
          <th>CLIENT</th><th>DONE</th><th>PENDING</th><th>SRC</th><th>OUT</th><th>SAVED</th>
        </tr></thead>
        <tbody id="rep-clients"></tbody>
      </table>
    </div>
  </div>
</div>

<script>
const STALL_TIMEOUT = {stall_to};
const NAS_DATA_ROOT = {repr(nas_data_root)};
const STATUS_COLORS = {{pending:'#b39ddb',active:'#f48fb1',done:'#80cbc4',failed:'#ef9a9a'}};
const STATUS_KR     = {{pending:'대기중',active:'인코딩',done:'완료',failed:'실패'}};
function nasDisplayPath(p) {{
  if (!p || !NAS_DATA_ROOT) return p;
  if (p.startsWith('/data/')) return NAS_DATA_ROOT + p.slice('/data'.length);
  if (p.startsWith('/data')) return NAS_DATA_ROOT + p.slice('/data'.length);
  return p;
}}
const PHASE_LABELS  = {{encoding:'▶ ENCODING',uploading:'▲ UPLOADING',verifying:'◎ VERIFYING'}};
const PHASE_CSS     = {{encoding:'phase-encoding',uploading:'phase-uploading',verifying:'phase-verifying'}};
const WORKER_COLORS = {{encoding:'#f48fb1',uploading:'#80cbc4',verifying:'#ce93d8',idle:'#444'}};

let _clientColors = {{}};
let _openDetail = null;

const S = {{
  page: 1, perPage: 50,
  sort: 'smart', order: 'desc',
  statusFilter: [],
  clientFilter: [],
  selectMode: false,
  selected: new Set(),
}};

// ── tab switching ─────────────────────────────────────────────────────────────
function switchTab(name) {{
  ['queue','report','settings'].forEach(t => {{
    document.getElementById('tab-'+t).style.display = t===name ? '' : 'none';
    document.getElementById('tab-btn-'+t).classList.toggle('active', t===name);
  }});
  if (name === 'settings') loadSettings();
  if (name === 'report')   loadReport();
}}

// ── report ────────────────────────────────────────────────────────────────────
const RES_COLORS  = {{'4K':'#f48fb1','1080p':'#ce93d8','720p':'#b39ddb','480p':'#80cbc4','<480p':'#90caf9','unknown':'#444'}};
const CODEC_COLORS= {{'h264':'#ce93d8','hevc':'#f48fb1','h265':'#f48fb1','av1':'#80cbc4','vp9':'#b39ddb','mpeg2video':'#ffcc80','wmv1':'#ef9a9a','wmv2':'#ef9a9a','unknown':'#444','':'#333'}};

function fmtGb(b) {{
  if (!b) return '0 GB';
  if (b >= 1e12) return (b/1e12).toFixed(1)+' TB';
  if (b >= 1e9)  return (b/1e9).toFixed(1)+' GB';
  return (b/1e6).toFixed(0)+' MB';
}}

async function loadReport() {{
  try {{
    const d = await fetch('/stats').then(r=>r.json());
    renderReport(d);
  }} catch(e) {{ console.error('loadReport', e); }}
}}

function renderReport(d) {{
  const total = (d.counts.pending||0)+(d.counts.active||0)+(d.counts.done||0)+(d.counts.failed||0);
  const saved = d.done.src_bytes - d.done.out_bytes;
  const savePct = d.done.src_bytes > 0 ? (saved/d.done.src_bytes*100).toFixed(1) : 0;

  document.getElementById('rep-saved').textContent    = fmtGb(saved);
  document.getElementById('rep-saved-pct').textContent = savePct+'% reduction · '+fmtGb(d.done.src_bytes)+' → '+fmtGb(d.done.out_bytes);
  document.getElementById('rep-done').textContent     = d.done.n.toLocaleString();
  document.getElementById('rep-done-of').textContent  = 'of '+total.toLocaleString()+' total';
  document.getElementById('rep-avg').textContent      = d.savings.avg ? d.savings.avg.toFixed(1)+'%' : '—';
  document.getElementById('rep-med').textContent      = d.savings.median ? 'median '+d.savings.median.toFixed(1)+'%  ·  range '+d.savings.min+'–'+d.savings.max+'%' : 'no data yet';
  document.getElementById('rep-hours').textContent    = (d.done.dur_secs/3600).toFixed(1);

  // resolution bars
  const maxRes = Math.max(...(d.pending.resolutions.map(r=>r.n)||[1]));
  document.getElementById('rep-res-bars').innerHTML = d.pending.resolutions.map(r => {{
    const pct = maxRes ? (r.n/maxRes*100).toFixed(1) : 0;
    const col = RES_COLORS[r.bucket] || '#555';
    return `<div class="bar-row">
      <span class="bar-label" style="color:${{col}}">${{r.bucket}}</span>
      <div class="bar-track"><div class="bar-fill" style="width:${{pct}}%;background:${{col}}"></div></div>
      <span class="bar-count" style="color:${{col}}">${{r.n.toLocaleString()}}</span>
      <span class="bar-bytes">${{fmtGb(r.bytes)}}</span>
    </div>`;
  }}).join('') || '<span style="color:#333">no pending</span>';

  // codec bars
  const maxC = Math.max(...(d.pending.codecs.map(c=>c.n)||[1]));
  document.getElementById('rep-codec-bars').innerHTML = d.pending.codecs.map(c => {{
    const pct = maxC ? (c.n/maxC*100).toFixed(1) : 0;
    const col = CODEC_COLORS[c.codec] || '#555';
    return `<div class="bar-row">
      <span class="bar-label" style="color:${{col}}">${{c.codec||'?'}}</span>
      <div class="bar-track"><div class="bar-fill" style="width:${{pct}}%;background:${{col}}"></div></div>
      <span class="bar-count" style="color:${{col}}">${{c.n.toLocaleString()}}</span>
    </div>`;
  }}).join('') || '<span style="color:#333">no pending</span>';

  // savings distribution
  const sv = d.savings;
  if (sv && sv.min != null) {{
    const span = sv.max - sv.min || 1;
    const avgOff = ((sv.avg - sv.min)/span*100).toFixed(1);
    const medOff = ((sv.median - sv.min)/span*100).toFixed(1);
    document.getElementById('rep-savings-dist').innerHTML = `
      <div class="savings-range">
        <div class="sr-pill"><div class="sr-pill-val">${{sv.min}}%</div><div class="sr-pill-lbl">MIN</div></div>
        <div class="sr-pill"><div class="sr-pill-val">${{sv.median}}%</div><div class="sr-pill-lbl">MEDIAN</div></div>
        <div class="sr-pill"><div class="sr-pill-val" style="color:#f48fb1">${{sv.avg}}%</div><div class="sr-pill-lbl">AVG</div></div>
        <div class="sr-pill"><div class="sr-pill-val">${{sv.max}}%</div><div class="sr-pill-lbl">MAX</div></div>
      </div>
      <div class="savings-bar-wrap">
        <div class="savings-bar-fill" style="left:0;width:100%"></div>
        <div class="savings-bar-avg" style="left:${{avgOff}}%" title="avg ${{sv.avg}}%"></div>
        <div class="savings-bar-marker" style="left:${{medOff}}%" title="median ${{sv.median}}%"></div>
      </div>
      <div style="display:flex;justify-content:space-between;font-size:9px;color:#444;margin-top:2px">
        <span>${{sv.min}}%</span><span style="color:#ce93d8">▲ avg ${{sv.avg}}%  ·  median ${{sv.median}}%</span><span>${{sv.max}}%</span>
      </div>`;
  }} else {{
    document.getElementById('rep-savings-dist').innerHTML = '<span style="color:#333">no completed jobs yet</span>';
  }}

  // client table
  document.getElementById('rep-clients').innerHTML = d.clients.map(c => {{
    const saved = c.done_src - c.done_out;
    return `<tr>
      <td>${{esc(c.client_name||'unknown')}}</td>
      <td style="color:#80cbc4">${{c.done}}</td>
      <td style="color:#b39ddb">${{c.pending}}</td>
      <td style="color:#555">${{fmtGb(c.done_src)}}</td>
      <td style="color:#555">${{fmtGb(c.done_out)}}</td>
      <td style="color:${{saved>0?'#80cbc4':'#444'}}">${{fmtGb(saved)}}</td>
    </tr>`;
  }}).join('');
}}

// ── sort ──────────────────────────────────────────────────────────────────────
function setSort(col) {{
  if (S.sort === col) {{
    S.order = S.order === 'desc' ? 'asc' : 'desc';
  }} else {{
    S.sort = col;
    S.order = 'desc';
  }}
  S.page = 1;
  updateSortHeaders();
  loadJobs();
}}

function updateSortHeaders() {{
  ['name','height','size','savings'].forEach(col => {{
    const th = document.getElementById('th-'+col);
    if (!th) return;
    const arrow = S.sort===col ? (S.order==='desc' ? ' ↓' : ' ↑') : '';
    const base = {{name:'FILE',height:'RES',size:'SIZE',savings:'SAVINGS'}}[col];
    th.textContent = base + arrow;
    th.classList.toggle('sort-active', S.sort===col);
  }});
}}

// ── status filter ─────────────────────────────────────────────────────────────
function setStatusFilter(statuses) {{
  S.statusFilter = statuses;
  S.page = 1;
  syncStatusPills();
  loadJobs();
}}

function toggleStatusPill(s) {{
  const idx = S.statusFilter.indexOf(s);
  if (idx >= 0) S.statusFilter.splice(idx, 1);
  else S.statusFilter.push(s);
  S.page = 1;
  syncStatusPills();
  loadJobs();
}}

function syncStatusPills() {{
  document.querySelectorAll('.spill').forEach(btn => {{
    const ds = btn.dataset.status;
    if (ds === '') btn.classList.toggle('active', S.statusFilter.length === 0);
    else btn.classList.toggle('active', S.statusFilter.includes(ds));
  }});
  ['pending','active','done','failed'].forEach(s => {{
    const card = document.getElementById('card-'+s);
    if (card) card.classList.toggle('active-filter', S.statusFilter.includes(s));
  }});
  const allCard = document.getElementById('card-all');
  if (allCard) allCard.classList.toggle('active-filter', S.statusFilter.length === 0);
}}

// ── client filter ─────────────────────────────────────────────────────────────
function toggleClientFilter(name) {{
  const idx = S.clientFilter.indexOf(name);
  if (idx >= 0) S.clientFilter.splice(idx, 1);
  else S.clientFilter.push(name);
  S.page = 1;
  syncClientChips();
  loadJobs();
}}

function syncClientChips() {{
  document.querySelectorAll('.chip').forEach(c => {{
    c.classList.toggle('active', S.clientFilter.includes(c.dataset.client));
  }});
}}

// ── pagination ────────────────────────────────────────────────────────────────
function setPage(p) {{ S.page = p; loadJobs(); }}

function renderPagination(data) {{
  const el = document.getElementById('pagination');
  if (data.pages <= 1) {{ el.innerHTML = ''; return; }}
  const total = data.total, pp = data.per_page, cur = data.page, pages = data.pages;
  const from = (cur-1)*pp+1, to = Math.min(cur*pp, total);
  let html = `<span class="page-info">${{from}}–${{to}} of ${{total}}</span>`;
  html += `<button class="ppage" ${{cur<=1?'disabled':''}} onclick="setPage(${{cur-1}})">‹ PREV</button>`;
  const lo = Math.max(1,cur-2), hi = Math.min(pages,cur+2);
  if (lo>1) html += `<button class="ppage" onclick="setPage(1)">1</button>${{lo>2?'<span class="page-info">…</span>':''}}`;
  for (let i=lo;i<=hi;i++) html += `<button class="ppage ${{i===cur?'active':''}}" onclick="setPage(${{i}})">${{i}}</button>`;
  if (hi<pages) html += `${{hi<pages-1?'<span class="page-info">…</span>':''}}<button class="ppage" onclick="setPage(${{pages}})">${{pages}}</button>`;
  html += `<button class="ppage" ${{cur>=pages?'disabled':''}} onclick="setPage(${{cur+1}})">NEXT ›</button>`;
  el.innerHTML = html;
}}

// ── data loading ──────────────────────────────────────────────────────────────
async function loadJobs() {{
  const p = new URLSearchParams({{sort:S.sort,order:S.order,page:S.page,per_page:S.perPage}});
  if (S.statusFilter.length) p.set('status', S.statusFilter.join(','));
  if (S.clientFilter.length) p.set('client', S.clientFilter.join(','));
  try {{
    const data = await fetch('/jobs?' + p).then(r => r.json());
    renderTable(data.jobs);
    renderPagination(data);
    updateCounts(data.counts);
  }} catch(e) {{ console.error('loadJobs', e); }}
}}

async function loadClients() {{
  try {{
    const clients = await fetch('/clients').then(r => r.json());
    _clientColors = {{}};
    clients.forEach(c => {{ _clientColors[c.name] = c.color; }});
    renderClientChips(clients);
  }} catch(e) {{}}
}}

async function loadControl() {{
  try {{
    const d = await fetch('/control').then(r=>r.json());
    ['run','drain','stop'].forEach(c => document.getElementById('btn-'+c).classList.remove('active'));
    const el = document.getElementById('btn-'+d.command);
    if (el) el.classList.add('active');
    document.getElementById('cmd-badge').textContent = d.command.toUpperCase();
  }} catch(e) {{}}
}}

async function loadLive() {{
  try {{
    const data = await fetch('/jobs/live').then(r=>r.json());
    const jobs = Object.values(data);
    const card = document.getElementById('live-card');
    if (!jobs.length) {{ card.classList.remove('active'); return; }}
    const j = jobs[0];
    card.classList.add('active');
    const phase = j.phase || 'encoding';
    const phEl = document.getElementById('live-phase');
    phEl.textContent = PHASE_LABELS[phase] || phase.toUpperCase();
    phEl.className = 'live-phase ' + (PHASE_CSS[phase] || 'phase-encoding');
    document.getElementById('live-file').textContent = j.file || '—';
    document.getElementById('live-worker').textContent = j.worker || '—';
    const bar = document.getElementById('live-bar');
    if (phase === 'encoding') {{
      bar.className = 'live-bar';
      bar.style.width = (j.percent||0) + '%';
      document.getElementById('live-pct').textContent = (j.percent||0).toFixed(1)+'%';
      document.getElementById('live-fps').textContent = j.fps ? j.fps.toFixed(1) : '—';
      document.getElementById('live-speed').textContent = j.speed || '—';
      document.getElementById('live-frame').textContent = j.frame ? j.frame.toLocaleString() : '—';
      document.getElementById('live-bitrate').textContent = j.bitrate || '—';
      document.getElementById('live-time').textContent = j.out_time ? j.out_time.split('.')[0] : '—';
    }} else {{
      bar.className = 'live-bar-ind'; bar.style.width = '';
    }}
  }} catch(e) {{}}
}}

async function loadWorkers() {{
  try {{
    const workers = await fetch('/workers').then(r=>r.json());
    const sec = document.getElementById('workers-section');
    const con = document.getElementById('workers-badges');
    if (!workers.length) {{ sec.style.display='none'; return; }}
    sec.style.display = 'block';
    const now = Date.now() / 1000;
    con.innerHTML = workers.map(w => {{
      const age = Math.round(now - w.last_seen);
      const dot = w.online ? `<span style="color:${{WORKER_COLORS[w.status]||'#555'}}">●</span>` : '<span style="color:#c62828">●</span>';
      const sc = w.online ? (WORKER_COLORS[w.status]||'#555') : '#c62828';
      const sl = w.online ? (w.status||'idle').toUpperCase() : 'DEAD';
      const ageStr = w.online ? `<span style="color:#444;font-size:9px">${{age}}s ago</span>` : `<span style="color:#c62828;font-size:9px">last seen ${{age}}s ago</span>`;
      const fl = w.current_file ? `<span style="color:#555;font-size:10px;max-width:200px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" title="${{w.current_file}}">${{esc(w.current_file)}}</span>` : '';
      return `<div style="background:#13132a;border:1px solid ${{w.online?'#1e1e3a':'#c6282844'}};border-radius:10px;padding:8px 14px;display:inline-flex;align-items:center;gap:8px">
        ${{dot}}<span style="color:#e0e0e0;font-size:12px">${{esc(w.name)}}</span>
        <span style="background:${{sc}}22;color:${{sc}};border:1px solid ${{sc}}44;padding:1px 8px;border-radius:20px;font-size:9px">${{sl}}</span>
        ${{ageStr}}${{fl}}</div>`;
    }}).join('');
  }} catch(e) {{}}
}}

async function loadSettings() {{
  try {{
    const d = await fetch('/settings').then(r=>r.json());
    document.getElementById('s-min-size').value    = d.min_size_mb || 0;
    document.getElementById('s-min-height').value  = d.min_height  || 0;
    document.getElementById('s-min-bitrate').value = d.min_bitrate_kbps || 0;
    setToggle('tog-hevc', d.skip_hevc === 'true');
    setToggle('tog-av1',  d.skip_av1  === 'true');
  }} catch(e) {{}}
  loadClientWeights();
  loadCompanionQueue();
}}

async function loadClientWeights() {{
  try {{
    const clients = await fetch('/clients').then(r=>r.json());
    const el = document.getElementById('client-weights-list');
    if (!clients.length) {{ el.innerHTML = '<span style="color:#444">no clients yet</span>'; return; }}
    el.innerHTML = clients.map(c => {{
      const w = c.weight || 5;
      const col = c.color || '#ce93d8';
      return `<div class="weight-row" data-client="${{esc(c.name)}}">
        <span class="weight-row-name" style="color:${{col}}">${{esc(c.name)}}</span>
        <input type="range" class="weight-slider" min="1" max="100" value="${{w}}"
          style="accent-color:${{col}}"
          oninput="updateWeightDisplay(this)"
          data-name="${{esc(c.name)}}">
        <span class="weight-val" id="wval-${{esc(c.name)}}" style="color:${{col}}">${{w}}</span>
        <span class="weight-pct" id="wpct-${{esc(c.name)}}"></span>
      </div>`;
    }}).join('');
    recalcWeightPcts();
  }} catch(e) {{}}
}}

function updateWeightDisplay(slider) {{
  const name = slider.dataset.name;
  document.getElementById('wval-'+name).textContent = slider.value;
  recalcWeightPcts();
}}

function recalcWeightPcts() {{
  const sliders = document.querySelectorAll('.weight-slider');
  const total = [...sliders].reduce((s, sl) => s + parseInt(sl.value), 0) || 1;
  sliders.forEach(sl => {{
    const pct = Math.round(parseInt(sl.value) / total * 100);
    const el = document.getElementById('wpct-'+sl.dataset.name);
    if (el) el.textContent = pct + '%';
  }});
}}

async function saveClientWeights() {{
  const sliders = document.querySelectorAll('.weight-slider');
  const payload = {{}};
  sliders.forEach(sl => {{ payload[sl.dataset.name] = parseInt(sl.value); }});
  await fetch('/clients/weights', {{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify(payload)}});
  const ok = document.getElementById('save-weights-ok');
  ok.style.opacity = 1;
  setTimeout(() => {{ ok.style.opacity = 0; }}, 2500);
}}

async function loadCompanionQueue() {{
  try {{
    const clients = await fetch('/clients').then(r=>r.json());
    const el = document.getElementById('companion-queue-list');
    const withQueue = clients.filter(c => c.queued_count > 0);
    if (!withQueue.length) {{
      el.innerHTML = '<span style="color:#444">no companion queue data — companion must send manifest</span>';
      return;
    }}
    el.innerHTML = withQueue.map(c => {{
      const col = c.color || '#ce93d8';
      return `<div style="margin-bottom:10px">
        <span style="color:${{col}};font-size:12px">${{esc(c.name)}}</span>
        <span style="color:#555;margin:0 8px">·</span>
        <span style="color:#e0e0e0">${{c.queued_count}} file${{c.queued_count!==1?'s':''}} queued for upload</span>
      </div>`;
    }}).join('');
  }} catch(e) {{}}
}}

// ── rendering ─────────────────────────────────────────────────────────────────
function updateCounts(counts) {{
  if (!counts) return;
  const total = (counts.pending||0) + (counts.active||0) + (counts.done||0) + (counts.failed||0);
  document.getElementById('cnt-total').textContent   = total;
  document.getElementById('cnt-pending').textContent = counts.pending||0;
  document.getElementById('cnt-active').textContent  = counts.active||0;
  document.getElementById('cnt-done').textContent    = counts.done||0;
  document.getElementById('cnt-failed').textContent  = counts.failed||0;
  const pct = total ? Math.round((counts.done||0)/total*100) : 0;
  document.getElementById('prog-bar').style.width = pct + '%';
  document.getElementById('prog-pct').textContent = pct + '% · ' + (counts.done||0) + ' / ' + total + ' files';
}}

function renderClientChips(clients) {{
  const con = document.getElementById('client-chips');
  const now = Date.now()/1000;
  con.innerHTML = clients.map(c => {{
    const ago = now - (c.last_seen||0);
    const dot = ago < 300 ? `<span style="color:${{c.color}}">●</span>` : ago < 3600 ? '<span style="color:#333">●</span>' : '';
    const qBadge = c.queued_count > 0
      ? `<span style="background:${{c.color}}33;border:1px solid ${{c.color}}55;border-radius:20px;padding:0 5px;font-size:9px" title="${{c.queued_count}} queued in companion">+${{c.queued_count}}</span>`
      : '';
    return `<div class="chip" data-client="${{esc(c.name)}}" onclick="toggleClientFilter('${{esc(c.name)}}')"
      style="background:${{c.color}}11;color:${{c.color}};border-color:${{c.color}}33">
      ${{dot}} ${{esc(c.name)}} <span style="opacity:.4;font-size:9px">${{c.uploads}}</span>${{qBadge}}</div>`;
  }}).join('');
  syncClientChips();
}}

function renderTable(jobs) {{
  const tbody = document.getElementById('jobs-tbody');
  if (!jobs.length) {{
    tbody.innerHTML = '<tr><td colspan="8" style="color:#444;text-align:center;padding:32px">no jobs found</td></tr>';
    return;
  }}
  tbody.innerHTML = jobs.map(renderRow).join('');
  // re-attach open detail if still relevant
  if (_openDetail) {{
    const tr = tbody.querySelector(`tr[data-id="${{_openDetail}}"]`);
    if (!tr) {{ _openDetail = null; }}
  }}
}}

function renderRow(job) {{
  const meta = job.source_meta ? tryParse(job.source_meta) : null;
  const res  = meta && meta.width && meta.height ? meta.width+'×'+meta.height : '—';
  const name = job.source_filename || (job.source_path ? job.source_path.split('/').pop() : job.id);
  const sc   = STATUS_COLORS[job.status] || '#eee';
  const sk   = STATUS_KR[job.status] || job.status;

  let sizeTxt = job.source_size ? fmtSz(job.source_size) : '—';
  let savings = '—';
  if (job.source_size > 0 && job.output_size > 0) {{
    const pct = (1 - job.output_size / job.source_size) * 100;
    const col = savingsColor(pct);
    savings = `<span style="color:${{col}}">${{fmtSz(job.output_size)}}</span> <span style="color:${{col}};font-size:10px;opacity:.8">-${{pct.toFixed(0)}}%</span>`;
  }}

  let progress = '<span style="color:#333">—</span>';
  if (job.status === 'active') {{
    const elapsed = Date.now()/1000 - (job.updated_at||0);
    const elMin = Math.floor(elapsed/60);
    const ageLabel = elMin >= 1 ? ` ${{elMin}}m` : '';
    const stalled = elapsed > STALL_TIMEOUT * 0.75 && job.percent < 1;
    const bc = stalled ? '#ef9a9a' : 'linear-gradient(90deg,#f48fb1,#ce93d8)';
    const sw = stalled ? '<span style="color:#ef9a9a;font-size:9px;margin-left:4px">⚠ STALLED</span>' : '';
    progress = `<div style="background:#1a1a2e;border-radius:20px;height:5px;margin-bottom:3px">
      <div style="width:${{job.percent.toFixed(0)}}%;background:${{bc}};height:5px;border-radius:20px"></div></div>
      <span style="font-size:11px">${{job.percent.toFixed(0)}}%${{ageLabel}}${{sw}}</span>`;
  }} else if (job.status === 'done') {{
    progress = '<span style="color:#80cbc4;font-size:11px">✓ done</span>';
  }} else if (job.status === 'failed') {{
    progress = '<span style="color:#ef9a9a;font-size:11px">✗ failed</span>';
  }}

  const cn = job.client_name || '';
  const cc = _clientColors[cn] || '#b39ddb';
  const clientBadge = cn
    ? `<span style="background:${{cc}}22;color:${{cc}};border:1px solid ${{cc}}55;padding:2px 8px;border-radius:20px;font-size:10px">${{esc(cn)}}</span>`
    : '<span style="color:#333">—</span>';

  const selCls = S.selected.has(job.id) ? ' selected' : '';
  return `<tr class="job-row${{selCls}}" data-id="${{job.id}}" data-status="${{job.status}}" onclick="toggleDetail(this,'${{job.id}}')">
    <td><span style="background:${{sc}}22;color:${{sc}};border:1px solid ${{sc}}55;padding:2px 8px;border-radius:20px;font-size:10px;letter-spacing:1px">${{sk}}</span></td>
    <td style="color:#e0e0e0;max-width:280px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" title="${{esc(job.source_path||'')}}">${{esc(name)}}</td>
    <td style="color:#666;font-size:11px">${{res}}</td>
    <td style="color:#b39ddb">${{sizeTxt}}</td>
    <td style="font-size:11px">${{savings}}</td>
    <td style="min-width:120px">${{progress}}</td>
    <td>${{clientBadge}}</td>
    <td style="color:#80cbc4;font-size:11px">${{esc(job.worker||'—')}}</td>
  </tr>`;
}}

// ── detail panel ──────────────────────────────────────────────────────────────
async function toggleDetail(tr, jobId) {{
  if (S.selectMode) {{ toggleSelect(jobId, tr); return; }}

  const existing = document.getElementById('detail-'+jobId);
  if (existing) {{ existing.remove(); _openDetail = null; return; }}
  document.querySelectorAll('.detail-row').forEach(r => r.remove());
  _openDetail = jobId;
  const job = await fetch('/jobs/'+jobId).then(r=>r.json()).catch(()=>({{}}));
  const src  = tryParse(job.source_meta);
  const out  = tryParse(job.output_meta);
  const chks = tryParse(job.verify_checks) || [];

  // For companion uploads: prefer client_path (full Mac path when companion sends it),
  // fall back to source_filename, and label the server temp copy separately.
  const isCompanion = job.source_filename && job.source_path &&
                      job.source_path.includes('/.transcode/uploads/');
  const displaySrc  = job.client_path || (isCompanion ? null : job.source_path) || job.source_path || '—';
  const srcLabel    = job.client_path ? 'source' : (isCompanion ? 'original name' : 'source');
  const srcDisplay  = job.client_path ? job.client_path
                    : (isCompanion ? (job.source_filename || job.source_path) : nasDisplayPath(job.source_path)) || '—';
  const outPath = nasDisplayPath(job.output_path) || '—';
  const serverCopyRow = isCompanion && !job.client_path
    ? '<span class="dk" style="color:#333">server copy</span><span style="color:#333;font-size:10px">' + esc(job.source_path||'') + '</span>'
    : '';
  const isDone    = job.status === 'done';
  const isPending = job.status === 'pending';

  const rescanBtn = `<button class="btn-rescan" id="rescan-${{jobId}}" onclick="rescanJob('${{jobId}}',this)">⟳ RESCAN</button>`;

  const actionsHtml = isDone
    ? `<div style="margin-top:12px;display:flex;align-items:center;gap:12px;flex-wrap:wrap">
        <label style="display:flex;align-items:center;gap:6px;cursor:pointer;color:#777;font-size:11px">
          <input type="checkbox" id="del-rename-${{jobId}}" style="accent-color:#ce93d8">
          rename output to original filename
        </label>
        <button class="btn-del-orig" onclick="deleteOriginal('${{jobId}}',this)">✕ DELETE ORIGINAL</button>
        ${{rescanBtn}}
       </div>`
    : isPending
    ? `<div style="margin-top:12px;display:flex;align-items:center;gap:12px;flex-wrap:wrap">
        <button class="btn-force-enc" onclick="forceEncode('${{jobId}}',this)">▶ FORCE ENCODE</button>
        <span style="color:#444;font-size:10px">sets control to RUN · brings to top of queue</span>
        ${{rescanBtn}}
       </div>`
    : `<div style="margin-top:12px">${{rescanBtn}}</div>`;

  const detail = document.createElement('tr');
  detail.id = 'detail-'+jobId;
  detail.className = 'detail-row';
  detail.innerHTML = `<td colspan="8"><div class="detail-panel">
    <div class="detail-section" style="grid-column:1/-1">
      <div class="detail-title">PATHS</div>
      <div class="path-box">
        <span class="dk">${{srcLabel}}</span>
        ${{isCompanion && !job.client_path
          ? `<span id="src-path-display-${{jobId}}" style="color:#b39ddb;user-select:all">${{esc(srcDisplay)}}
               <button onclick="editClientPath('${{jobId}}')" style="background:none;border:1px solid #b39ddb44;color:#b39ddb77;font-size:9px;padding:1px 6px;border-radius:4px;cursor:pointer;margin-left:6px;font-family:inherit">✎ set path</button>
             </span>
             <span id="src-path-edit-${{jobId}}" style="display:none;grid-column:2">
               <input id="src-path-input-${{jobId}}" type="text" placeholder="/Users/you/path/to/file.mp4"
                 style="background:#0d0d1a;border:1px solid #b39ddb55;color:#b39ddb;font-family:inherit;font-size:11px;padding:4px 8px;border-radius:6px;width:520px;max-width:100%">
               <button onclick="saveClientPath('${{jobId}}')" style="background:#b39ddb22;border:1px solid #b39ddb55;color:#b39ddb;font-family:inherit;font-size:9px;padding:4px 10px;border-radius:6px;cursor:pointer;margin-left:6px">save</button>
               <button onclick="cancelEditPath('${{jobId}}')" style="background:none;border:none;color:#555;font-family:inherit;font-size:9px;padding:4px 6px;cursor:pointer">cancel</button>
             </span>`
          : `<span style="color:#b39ddb;user-select:all">${{esc(srcDisplay)}}</span>`
        }}
        <span class="dk">output</span><span style="color:#80cbc4;user-select:all">${{esc(outPath)}}</span>
        ${{serverCopyRow}}
      </div>
      ${{actionsHtml}}
    </div>
    <div class="detail-section">
      <div class="detail-title">SOURCE</div>
      <div class="detail-grid">${{metaGrid(src, job.source_size, null)}}</div>
    </div>
    <div class="detail-section">
      <div class="detail-title">OUTPUT ${{job.verify_status ? '· '+job.verify_status.toUpperCase() : ''}}</div>
      <div class="detail-grid">${{metaGrid(out, job.output_size, job.source_size)}}</div>
    </div>
    <div class="detail-section checks">
      <div class="detail-title">VERIFICATION CHECKS</div>
      ${{checksHtml(chks)}}
    </div>
  </div></td>`;
  tr.parentNode.insertBefore(detail, tr.nextSibling);
}}

// ── select mode ───────────────────────────────────────────────────────────────
function toggleSelectMode() {{
  S.selectMode ? exitSelectMode() : enterSelectMode();
}}

function enterSelectMode() {{
  S.selectMode = true;
  S.selected.clear();
  document.getElementById('btn-select').classList.add('active');
  document.getElementById('btn-select').style.opacity = '1';
  document.querySelectorAll('.detail-row').forEach(r => r.remove());
  _openDetail = null;
  updateBulkBar();
}}

function exitSelectMode() {{
  S.selectMode = false;
  S.selected.clear();
  document.getElementById('btn-select').classList.remove('active');
  document.getElementById('btn-select').style.opacity = '';
  document.querySelectorAll('.job-row.selected').forEach(r => r.classList.remove('selected'));
  document.getElementById('bulk-bar').style.display = 'none';
}}

function toggleSelect(jobId, tr) {{
  if (S.selected.has(jobId)) {{
    S.selected.delete(jobId);
    tr.classList.remove('selected');
  }} else {{
    // only done jobs can be selected for bulk delete
    if (tr.dataset.status !== 'done') return;
    S.selected.add(jobId);
    tr.classList.add('selected');
  }}
  updateBulkBar();
}}

function updateBulkBar() {{
  const bar = document.getElementById('bulk-bar');
  const n = S.selected.size;
  if (!S.selectMode) {{ bar.style.display = 'none'; return; }}
  bar.style.display = 'flex';
  document.getElementById('bulk-count').textContent = n + ' selected';
  bar.querySelector('.btn-del-orig').disabled = n === 0;
}}

// ── delete original ───────────────────────────────────────────────────────────
async function deleteOriginal(jobId, btn) {{
  const rename = document.getElementById('del-rename-'+jobId)?.checked || false;
  if (!confirm('Delete the original source file' + (rename ? ' and rename output to original filename' : '') + '?')) return;
  btn.disabled = true;
  btn.textContent = '…';
  try {{
    const r = await fetch('/jobs/'+jobId+'/delete-original', {{
      method:'POST', headers:{{'Content-Type':'application/json'}},
      body: JSON.stringify({{rename}})
    }}).then(r=>r.json());
    if (r.ok) {{
      btn.textContent = '✓ deleted';
      btn.style.color = '#80cbc4';
      btn.style.borderColor = '#80cbc455';
    }} else {{
      btn.textContent = '✗ error';
      btn.disabled = false;
    }}
  }} catch(e) {{
    btn.textContent = '✗ error';
    btn.disabled = false;
  }}
}}

async function bulkDeleteOriginal() {{
  const ids = [...S.selected];
  const rename = document.getElementById('bulk-rename').checked;
  if (!ids.length) return;
  if (!confirm('Delete original source files for ' + ids.length + ' job(s)' + (rename ? ' and rename outputs' : '') + '?')) return;
  const btn = document.querySelector('#bulk-bar .btn-del-orig');
  btn.disabled = true;
  btn.textContent = '…';
  try {{
    const r = await fetch('/jobs/bulk-delete-original', {{
      method:'POST', headers:{{'Content-Type':'application/json'}},
      body: JSON.stringify({{ids, rename}})
    }}).then(r=>r.json());
    const ok = (r.results||[]).filter(x=>x.ok).length;
    btn.textContent = '✓ done ('+ok+'/'+ids.length+')';
    exitSelectMode();
    loadJobs();
  }} catch(e) {{
    btn.textContent = '✗ error';
    btn.disabled = false;
  }}
}}

// ── bulk rescan ───────────────────────────────────────────────────────────────
async function bulkRescan() {{
  const ids = [...S.selected];
  if (!ids.length) return;
  const btn = document.getElementById('bulk-rescan-btn');
  btn.disabled = true;
  btn.textContent = '⟳ scanning ' + ids.length + '…';
  try {{
    const r = await fetch('/jobs/bulk-rescan', {{
      method:'POST', headers:{{'Content-Type':'application/json'}},
      body: JSON.stringify({{ids}})
    }}).then(r=>r.json());
    const updated = Object.values(r.results||{{}}).filter(x => x.updated && x.updated.length > 0).length;
    btn.textContent = '✓ ' + updated + '/' + ids.length + ' updated';
    setTimeout(() => {{ exitSelectMode(); loadJobs(); }}, 900);
  }} catch(e) {{
    btn.textContent = '✗ error';
    btn.disabled = false;
  }}
}}

// ── force encode ──────────────────────────────────────────────────────────────
async function forceEncode(jobId, btn) {{
  btn.disabled = true;
  btn.textContent = '…';
  try {{
    const r = await fetch('/jobs/'+jobId+'/force-encode', {{method:'POST'}}).then(r=>r.json());
    if (r.ok) {{
      btn.textContent = '✓ queued — control set to RUN';
      btn.style.color = '#80cbc4';
      // sync control badge
      document.getElementById('cmd-badge').textContent = 'RUN';
      ['run','drain','stop'].forEach(c => document.getElementById('btn-'+c).classList.remove('active'));
      document.getElementById('btn-run').classList.add('active');
      document.querySelectorAll('.detail-row').forEach(r => r.remove());
      _openDetail = null;
      loadJobs();
    }}
  }} catch(e) {{
    btn.textContent = '✗ error';
    btn.disabled = false;
  }}
}}

// ── inline path edit ──────────────────────────────────────────────────────────
function editClientPath(jobId) {{
  document.getElementById('src-path-display-'+jobId).style.display = 'none';
  const edit = document.getElementById('src-path-edit-'+jobId);
  edit.style.display = 'inline-flex';
  edit.style.alignItems = 'center';
  const inp = document.getElementById('src-path-input-'+jobId);
  inp.focus();
  inp.addEventListener('keydown', e => {{
    if (e.key === 'Enter') saveClientPath(jobId);
    if (e.key === 'Escape') cancelEditPath(jobId);
  }});
}}

function cancelEditPath(jobId) {{
  document.getElementById('src-path-display-'+jobId).style.display = '';
  document.getElementById('src-path-edit-'+jobId).style.display = 'none';
}}

async function saveClientPath(jobId) {{
  const val = document.getElementById('src-path-input-'+jobId).value.trim();
  if (!val) return;
  const r = await fetch('/jobs/'+jobId+'/set-path', {{
    method:'POST', headers:{{'Content-Type':'application/json'}},
    body: JSON.stringify({{client_path: val}})
  }}).then(r=>r.json()).catch(()=>({{ok:false}}));
  if (r.ok) {{
    // Refresh detail panel to show the saved path
    document.querySelectorAll('.detail-row').forEach(r => r.remove());
    _openDetail = null;
    const row = document.querySelector('[data-id="'+jobId+'"]');
    if (row) toggleDetail(row, jobId);
  }}
}}

// ── rescan ─────────────────────────────────────────────────────────────────────
async function rescanJob(jobId, btn) {{
  btn.disabled = true;
  btn.textContent = '⟳ scanning…';
  try {{
    const r = await fetch('/jobs/'+jobId+'/rescan', {{method:'POST'}}).then(r=>r.json());
    if (r.ok) {{
      if (r.updated && r.updated.length > 0) {{
        btn.textContent = '✓ ' + r.updated.join(' · ');
        btn.style.color = '#80cbc4';
        // Refresh the detail panel to show updated metadata
        setTimeout(() => {{
          document.querySelectorAll('.detail-row').forEach(r => r.remove());
          _openDetail = null;
          const row = document.querySelector('[data-id="'+jobId+'"]');
          if (row) toggleDetail(row, jobId);
          loadJobs();
        }}, 800);
      }} else {{
        btn.textContent = '— no files found';
        btn.style.color = '#555';
        setTimeout(() => {{
          btn.textContent = '⟳ RESCAN';
          btn.style.color = '';
          btn.disabled = false;
        }}, 2500);
      }}
    }} else {{
      btn.textContent = '✗ ' + (r.detail || 'error');
      btn.disabled = false;
    }}
  }} catch(e) {{
    btn.textContent = '✗ error';
    btn.disabled = false;
  }}
}}

function metaGrid(m, sizeBytes, refBytes) {{
  if (!m) return '<span style="color:#444">no data yet</span>';
  const rows = [
    ['size', sizeBadge(sizeBytes, refBytes)],
    ['codec', m.video_codec||'—'],
    ['resolution', m.width&&m.height ? m.width+'×'+m.height : '—'],
    ['duration', fmtDur(m.duration)],
    ['fps', m.fps ? m.fps.toFixed(2) : '—'],
    ['frames', m.frames ? m.frames.toLocaleString() : '—'],
    ['bitrate', fmtBr(m.bitrate)],
    ['audio', (m.audio_codecs||[]).join(', ')||'—'],
    ['streams', m.stream_count||'—'],
  ];
  return rows.map(([k,v])=>`<span class="dk">${{k}}</span><span class="dv">${{v}}</span>`).join('');
}}

function checksHtml(checks) {{
  if (!checks||!checks.length) return '<span style="color:#444">no verify data</span>';
  return checks.map(c => {{
    const icon  = c.pass ? '<span class="check-pass">✓</span>' : '<span class="check-fail">✗</span>';
    const score = c.score!=null ? `<span class="check-score">${{(c.score*100).toFixed(0)}}%</span>` : '';
    return `<div class="check-row">${{icon}}<span class="check-name">${{c.name}}</span><span class="check-detail">${{c.detail||''}}</span>${{score}}</div>`;
  }}).join('');
}}

// ── settings ──────────────────────────────────────────────────────────────────
function toggleSetting(id) {{
  const el = document.getElementById(id);
  el.classList.toggle('on');
  document.getElementById(id+'-label').textContent = el.classList.contains('on') ? 'ON' : 'OFF';
}}

function setToggle(id, val) {{
  const el = document.getElementById(id);
  el.classList.toggle('on', val);
  document.getElementById(id+'-label').textContent = val ? 'ON' : 'OFF';
}}

async function saveSettings() {{
  const payload = {{
    min_size_mb:      document.getElementById('s-min-size').value,
    min_height:       document.getElementById('s-min-height').value,
    min_bitrate_kbps: document.getElementById('s-min-bitrate').value,
    skip_hevc: document.getElementById('tog-hevc').classList.contains('on') ? 'true' : 'false',
    skip_av1:  document.getElementById('tog-av1').classList.contains('on')  ? 'true' : 'false',
  }};
  await fetch('/settings', {{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify(payload)}});
  const ok = document.getElementById('save-ok');
  ok.style.opacity = 1;
  setTimeout(() => {{ ok.style.opacity = 0; }}, 2500);
}}

// ── control ───────────────────────────────────────────────────────────────────
async function ctrl(cmd) {{
  ['run','drain','stop'].forEach(c => document.getElementById('btn-'+c).classList.remove('active'));
  const el = document.getElementById('btn-'+cmd);
  if (el) el.classList.add('active');
  document.getElementById('cmd-badge').textContent = cmd.toUpperCase();
  await fetch('/control/'+cmd, {{method:'POST'}});
}}

async function post(url) {{
  await fetch(url, {{method:'POST'}});
}}

async function postAndRefresh(url) {{
  await post(url);
  await loadJobs();
}}

async function toggleNasDrain() {{
  const btn = document.getElementById('btn-nas-drain');
  const paused = btn.classList.contains('active');
  const next = paused ? 'false' : 'true';
  await fetch('/settings', {{method:'POST', headers:{{'Content-Type':'application/json'}}, body: JSON.stringify({{nas_drain: next}})}});
  btn.classList.toggle('active', !paused);
  btn.textContent = !paused ? '▶ NAS SCAN' : '⏸ NAS SCAN';
}}

async function initNasDrain() {{
  try {{
    const s = await fetch('/settings').then(r=>r.json());
    const paused = s.nas_drain === 'true';
    const btn = document.getElementById('btn-nas-drain');
    btn.classList.toggle('active', paused);
    btn.textContent = paused ? '▶ NAS SCAN' : '⏸ NAS SCAN';
  }} catch(e) {{}}
}}

// ── helpers ───────────────────────────────────────────────────────────────────
function tryParse(s) {{
  if (!s) return null;
  try {{ return JSON.parse(s); }} catch(e) {{ return null; }}
}}

function esc(s) {{
  return String(s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}}

function fmtDur(s) {{
  s = Math.round(s||0);
  const h=Math.floor(s/3600), m=Math.floor((s%3600)/60), ss=s%60;
  return h ? h+'h'+String(m).padStart(2,'0') : m+'m'+String(ss).padStart(2,'0')+'s';
}}

function fmtBr(bps) {{
  if (!bps) return '—';
  const mbps = bps/1e6;
  return mbps >= 1 ? mbps.toFixed(1)+' Mbps' : Math.round(bps/1e3)+' kbps';
}}

function fmtSz(b) {{
  if (!b) return '—';
  return b >= 1e9 ? (b/1e9).toFixed(2)+' GB' : (b/1e6).toFixed(0)+' MB';
}}

function savingsColor(pct) {{
  if (pct >= 65) return '#80cbc4';
  if (pct >= 45) return '#a5d6a7';
  if (pct >= 25) return '#ffcc80';
  return '#ef9a9a';
}}

function sizeBadge(bytes, refBytes) {{
  if (!bytes) return '—';
  const s = fmtSz(bytes);
  if (!refBytes) return `<span style="color:#e0e0e0">${{s}}</span>`;
  const pct = (1 - bytes / refBytes) * 100;
  const col = savingsColor(pct);
  const sign = pct >= 0 ? '-' : '+';
  return `<span style="color:${{col}}">${{s}}</span> <span style="color:${{col}};font-size:10px;opacity:.8">${{sign}}${{Math.abs(pct).toFixed(0)}}%</span>`;
}}

// ── init + polling ────────────────────────────────────────────────────────────
function init() {{
  loadJobs();
  loadClients();
  loadControl();
  loadLive();
  loadWorkers();
  updateSortHeaders();
  initNasDrain();
}}

setInterval(loadLive,    3000);
setInterval(loadWorkers, 5000);
setInterval(loadControl, 5000);
setInterval(() => {{ if (!_openDetail) loadJobs(); }}, 15000);

init();
</script>
</body></html>"""
