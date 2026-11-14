#!/usr/bin/env python3
"""Explicit Hub transfers. Credentials use the normal Hugging Face login store."""
import argparse
from pathlib import Path

def main():
    p = argparse.ArgumentParser()
    p.add_argument("mode", choices=["pull", "push"])
    p.add_argument("repo")
    p.add_argument("path", type=Path)
    a = p.parse_args()
    from huggingface_hub import HfApi, snapshot_download
    from huggingface_hub.utils import validate_repo_id
    validate_repo_id(a.repo)
    if a.mode == "pull":
        if a.path.exists() and any(a.path.iterdir()):
            raise ValueError("Download destination must be empty")
        snapshot_download(repo_id=a.repo, repo_type="dataset", local_dir=a.path)
        if not (a.path / "meta/info.json").is_file():
            raise ValueError("Downloaded repository is not a LeRobot dataset")
    else:
        if not (a.path / "meta/info.json").is_file():
            raise ValueError("Upload source is not a LeRobot dataset")
        api = HfApi()
        api.create_repo(repo_id=a.repo, repo_type="dataset", private=True, exist_ok=True)
        api.upload_folder(repo_id=a.repo, repo_type="dataset", folder_path=a.path,
                          ignore_patterns=[".staging/**", ".cache/**", "*.tmp", ".dorobot-writer.lock", "meta/.dorobot-commit*"],
                          commit_message="Upload dataset from DoRobot Studio")

if __name__ == "__main__":
    main()
