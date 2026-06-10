#!/usr/bin/env python3
"""End-to-end test for the complete Enkodu flow.

This test verifies the entire pipeline:
1. Upload a video via resumable upload
2. Poll until job is done
3. Verify server-side checksum
4. Download output with Range headers
5. Verify local checksum

Requirements:
    pip install requests

Usage:
    python3 queue/test_e2e.py http://localhost:8000 /path/to/test/video.mp4

The test video should be a small (e.g., 10MB) h264/h265 file.
"""

import hashlib
import os
import sys
import time

import requests


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def format_size(bytes_size: int) -> str:
    if bytes_size >= 1_000_000_000:
        return f"{bytes_size / 1e9:.2f} GB"
    elif bytes_size >= 1_000_000:
        return f"{bytes_size / 1e6:.1f} MB"
    elif bytes_size >= 1_000:
        return f"{bytes_size / 1e3:.0f} KB"
    else:
        return f"{bytes_size} B"


class E2ETest:
    def __init__(self, base_url: str, video_path: str):
        self.base_url = base_url.rstrip("/")
        self.video_path = video_path
        self.upload_id = None
        self.job_id = None
        self.chunk_size = None

    def run(self) -> bool:
        print("=" * 60)
        print(f"Enkodu E2E Test")
        print(f"Server: {self.base_url}")
        print(f"Video:  {self.video_path} ({format_size(os.path.getsize(self.video_path))})")
        print("=" * 60)

        try:
            self.step_1_health_check()
            self.step_2_upload_video()
            self.step_3_poll_until_done()
            self.step_4_verify_server_checksum()
            self.step_5_download_output()
            self.step_6_verify_local_checksum()
            print("\n" + "=" * 60)
            print("✅ ALL STEPS PASSED")
            print("=" * 60)
            return True
        except AssertionError as e:
            print(f"\n❌ FAILED: {e}")
            return False
        except Exception as e:
            print(f"\n💥 ERROR: {e}")
            return False

    def step_1_health_check(self):
        print("\n[1/6] Health check...")
        resp = requests.get(f"{self.base_url}/healthz")
        assert resp.status_code == 200, f"Health check failed: {resp.status_code}"
        data = resp.json()
        assert data.get("ok") is True, "Health check returned ok=False"
        print(f"  ✅ Server healthy (version: {data.get('version', 'unknown')})")

    def step_2_upload_video(self):
        print("\n[2/6] Resumable upload...")
        total_size = os.path.getsize(self.video_path)
        filename = os.path.basename(self.video_path)

        # Start
        resp = requests.post(
            f"{self.base_url}/jobs/upload/resumable/start",
            json={"filename": filename, "filepath": self.video_path, "total_size": total_size},
        )
        assert resp.status_code == 200, f"Upload start failed: {resp.status_code}"
        data = resp.json()
        self.upload_id = data["upload_id"]
        self.chunk_size = data["chunk_size"]
        print(f"  ✅ Upload started: {self.upload_id}, chunk_size={self.chunk_size}")

        # Send chunks
        uploaded = 0
        chunk_num = 0
        with open(self.video_path, "rb") as f:
            while uploaded < total_size:
                end = min(uploaded + self.chunk_size - 1, total_size - 1)
                chunk = f.read(self.chunk_size)
                resp = requests.put(
                    f"{self.base_url}/jobs/upload/resumable/{self.upload_id}/chunk",
                    headers={"Content-Range": f"bytes {uploaded}-{end}/{total_size}"},
                    data=chunk,
                )
                assert resp.status_code == 200, f"Chunk {chunk_num} failed: {resp.status_code}"
                uploaded += len(chunk)
                chunk_num += 1
                if chunk_num % 10 == 0 or uploaded >= total_size:
                    print(f"  📤 Chunk {chunk_num}: {format_size(uploaded)} / {format_size(total_size)}")

        # Finish
        resp = requests.post(
            f"{self.base_url}/jobs/upload/resumable/{self.upload_id}/finish"
        )
        assert resp.status_code == 200, f"Upload finish failed: {resp.status_code}"
        data = resp.json()
        self.job_id = data["job_id"]
        print(f"  ✅ Upload complete: job_id={self.job_id}")

    def step_3_poll_until_done(self):
        print("\n[3/6] Polling job status...")
        max_polls = 120  # 10 minutes at 5s intervals
        for i in range(max_polls):
            resp = requests.get(f"{self.base_url}/jobs/{self.job_id}")
            assert resp.status_code == 200, f"Poll failed: {resp.status_code}"
            data = resp.json()
            status = data["status"]
            percent = data.get("percent", 0)

            if status == "done":
                print(f"  ✅ Job done (polls: {i+1})")
                return
            elif status == "failed":
                error = data.get("error", "unknown")
                raise AssertionError(f"Job failed: {error}")

            if i % 6 == 0:  # Print every 30s
                print(f"  ⏳ Status: {status} ({percent:.0f}%)")
            time.sleep(5)

        raise AssertionError("Job did not complete within 10 minutes")

    def step_4_verify_server_checksum(self):
        print("\n[4/6] Verifying server checksum...")
        resp = requests.get(f"{self.base_url}/jobs/{self.job_id}/checksum")
        assert resp.status_code == 200, f"Checksum fetch failed: {resp.status_code}"
        data = resp.json()
        output_sha256 = data.get("output_sha256")
        assert output_sha256, "Server did not return output_sha256"
        print(f"  ✅ Server checksum: {output_sha256[:16]}...")

    def step_5_download_output(self):
        print("\n[5/6] Downloading output with Range headers...")
        resp = requests.get(f"{self.base_url}/jobs/{self.job_id}/output")
        assert resp.status_code in (200, 404), f"Output check failed: {resp.status_code}"
        if resp.status_code == 404:
            print("  ⚠️ Output not ready yet, waiting 5s...")
            time.sleep(5)
            resp = requests.get(f"{self.base_url}/jobs/{self.job_id}/output")
            assert resp.status_code == 200, f"Output still not ready: {resp.status_code}"

        total_size = len(resp.content)
        assert "Accept-Ranges" in resp.headers, "Server does not accept ranges"
        print(f"  ✅ Full download: {format_size(total_size)}")

        # Now download in chunks
        chunk_size = 1 << 20
        downloaded = b""
        offset = 0
        chunk_num = 0

        while offset < total_size:
            end = min(offset + chunk_size - 1, total_size - 1)
            resp = requests.get(
                f"{self.base_url}/jobs/{self.job_id}/output",
                headers={"Range": f"bytes={offset}-{end}"},
            )
            assert resp.status_code == 206, f"Range request failed: {resp.status_code}"
            downloaded += resp.content
            offset += len(resp.content)
            chunk_num += 1
            if chunk_num % 10 == 0 or offset >= total_size:
                print(f"  📥 Range chunk {chunk_num}: {format_size(offset)} / {format_size(total_size)}")

        assert len(downloaded) == total_size, "Reassembled size mismatch"
        self.downloaded_data = downloaded
        print(f"  ✅ Reassembled: {format_size(len(downloaded))}")

    def step_6_verify_local_checksum(self):
        print("\n[6/6] Verifying local checksum...")
        local_sha256 = hashlib.sha256(self.downloaded_data).hexdigest()

        resp = requests.get(f"{self.base_url}/jobs/{self.job_id}/checksum")
        server_sha256 = resp.json().get("output_sha256", "")

        assert local_sha256 == server_sha256, \
            f"Checksum mismatch!\nLocal:  {local_sha256}\nServer: {server_sha256}"
        print(f"  ✅ Checksum match: {local_sha256[:16]}...")


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <server_url> <video_path>")
        print(f"Example: {sys.argv[0]} http://localhost:8000 ~/Movies/test.mp4")
        sys.exit(1)

    server_url = sys.argv[1]
    video_path = sys.argv[2]

    if not os.path.exists(video_path):
        print(f"Error: Video file not found: {video_path}")
        sys.exit(1)

    test = E2ETest(server_url, video_path)
    success = test.run()
    sys.exit(0 if success else 1)
