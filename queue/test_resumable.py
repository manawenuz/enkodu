#!/usr/bin/env python3
"""Integration tests for resumable upload/download endpoints.

Run these against a running queue server:
    python3 queue/test_resumable.py http://localhost:8000

Requirements:
    pip install requests
"""

import hashlib
import io
import sys
import tempfile
import time
import os

import requests


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


class TestResumableUpload:
    def __init__(self, base_url: str):
        self.base_url = base_url.rstrip("/")
        self.upload_id = None
        self.chunk_size = None

    def test_start_upload(self):
        """Test resumable upload start."""
        resp = requests.post(
            f"{self.base_url}/jobs/upload/resumable/start",
            json={"filename": "test.mp4", "filepath": "/tmp/test.mp4", "total_size": 16_777_216},
        )
        assert resp.status_code == 200, f"Expected 200, got {resp.status_code}: {resp.text}"
        data = resp.json()
        assert "upload_id" in data
        assert "chunk_size" in data
        self.upload_id = data["upload_id"]
        self.chunk_size = data["chunk_size"]
        print(f"✓ start upload: upload_id={self.upload_id}, chunk_size={self.chunk_size}")

    def test_upload_chunks(self):
        """Test chunked upload."""
        assert self.upload_id, "Must call test_start_upload first"
        chunk_size = self.chunk_size
        total_size = 16_777_216
        uploaded = 0
        chunk_num = 0

        while uploaded < total_size:
            end = min(uploaded + chunk_size - 1, total_size - 1)
            chunk = bytes(0xAB for _ in range(end - uploaded + 1))
            resp = requests.put(
                f"{self.base_url}/jobs/upload/resumable/{self.upload_id}/chunk",
                headers={"Content-Range": f"bytes {uploaded}-{end}/{total_size}"},
                data=chunk,
            )
            assert resp.status_code == 200, f"Chunk {chunk_num} failed: {resp.status_code} {resp.text}"
            data = resp.json()
            assert data["ok"] is True
            uploaded = data["received"]
            chunk_num += 1
            print(f"✓ chunk {chunk_num}: uploaded {uploaded}/{total_size}")

    def test_finish_upload(self):
        """Test finalize upload."""
        assert self.upload_id, "Must call test_start_upload first"
        resp = requests.post(
            f"{self.base_url}/jobs/upload/resumable/{self.upload_id}/finish"
        )
        assert resp.status_code == 200, f"Finish failed: {resp.status_code} {resp.text}"
        data = resp.json()
        assert "job_id" in data
        print(f"✓ finish upload: job_id={data['job_id']}")
        return data["job_id"]

    def test_resume_upload(self):
        """Test resuming upload from partial state."""
        # Start a new upload
        self.test_start_upload()
        chunk_size = self.chunk_size
        total_size = 16_777_216
        uploaded = 0
        chunk_num = 0
        stop_at = 2  # stop after 2 chunks

        while uploaded < total_size and chunk_num < stop_at:
            end = min(uploaded + chunk_size - 1, total_size - 1)
            chunk = bytes(0xCD for _ in range(end - uploaded + 1))
            resp = requests.put(
                f"{self.base_url}/jobs/upload/resumable/{self.upload_id}/chunk",
                headers={"Content-Range": f"bytes {uploaded}-{end}/{total_size}"},
                data=chunk,
            )
            assert resp.status_code == 200
            data = resp.json()
            uploaded = data["received"]
            chunk_num += 1

        print(f"✓ paused upload at {uploaded}/{total_size}")

        # Resume by sending remaining chunks
        while uploaded < total_size:
            end = min(uploaded + chunk_size - 1, total_size - 1)
            chunk = bytes(0xCD for _ in range(end - uploaded + 1))
            resp = requests.put(
                f"{self.base_url}/jobs/upload/resumable/{self.upload_id}/chunk",
                headers={"Content-Range": f"bytes {uploaded}-{end}/{total_size}"},
                data=chunk,
            )
            assert resp.status_code == 200
            data = resp.json()
            uploaded = data["received"]
            chunk_num += 1

        job_id = self.test_finish_upload()
        print(f"✓ resumed upload complete: {job_id}")


class TestResumableDownload:
    def __init__(self, base_url: str):
        self.base_url = base_url.rstrip("/")

    def test_download_with_range(self, job_id: str):
        """Test download with HTTP Range header."""
        # First, get full download to check size
        resp = requests.get(f"{self.base_url}/jobs/{job_id}/output")
        assert resp.status_code in (200, 404)
        if resp.status_code == 404:
            print("⚠ output not ready yet, skipping range download test")
            return

        total_size = len(resp.content)
        assert "Accept-Ranges" in resp.headers
        print(f"✓ full download: {total_size} bytes")

        # Now download in chunks
        chunk_size = 1 << 20
        downloaded = b""
        offset = 0

        while offset < total_size:
            end = min(offset + chunk_size - 1, total_size - 1)
            resp = requests.get(
                f"{self.base_url}/jobs/{job_id}/output",
                headers={"Range": f"bytes={offset}-{end}"},
            )
            assert resp.status_code == 206, f"Range request failed: {resp.status_code}"
            assert "Content-Range" in resp.headers
            downloaded += resp.content
            offset += len(resp.content)
            print(f"✓ range download: {offset}/{total_size}")

        assert len(downloaded) == total_size
        print(f"✓ reassembled download: {total_size} bytes match")

    def test_checksum(self, job_id: str):
        """Test checksum endpoint."""
        resp = requests.get(f"{self.base_url}/jobs/{job_id}/checksum")
        assert resp.status_code == 200
        data = resp.json()
        assert "job_id" in data
        assert "status" in data
        print(f"✓ checksum: job_id={data['job_id']}, status={data['status']}")
        if data.get("output_sha256"):
            print(f"  output_sha256={data['output_sha256'][:16]}...")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <server_url>")
        print(f"Example: {sys.argv[0]} http://localhost:8000")
        sys.exit(1)

    base_url = sys.argv[1]
    print(f"Testing server at {base_url}")
    print("=" * 50)

    # Test resumable upload
    upload_test = TestResumableUpload(base_url)
    upload_test.test_start_upload()
    upload_test.test_upload_chunks()
    job_id = upload_test.test_finish_upload()
    print()

    # Test resume
    upload_test.test_resume_upload()
    print()

    # Test download
    download_test = TestResumableDownload(base_url)
    download_test.test_download_with_range(job_id)
    print()

    # Test checksum
    download_test.test_checksum(job_id)
    print()

    print("=" * 50)
    print("All tests passed!")
