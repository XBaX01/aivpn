import argparse
import os
import posixpath
import socket
import stat
import sys
from pathlib import Path

import paramiko


DEFAULT_REMOTE_DIR = "/root/aivpn"
UPLOAD_FILES = [
    "docker-compose.yml",
    "Dockerfile.prebuilt",
    "deploy-server-release.sh",
    "config/server.json",
    "releases/aivpn-server-linux-x86_64",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Upload local AIVPN deploy artifacts and run the fast server deploy remotely.")
    parser.add_argument("--host", required=True)
    parser.add_argument("--user", default="root")
    parser.add_argument("--password", required=True)
    parser.add_argument("--remote-dir", default=DEFAULT_REMOTE_DIR)
    parser.add_argument("--port", type=int, default=22)
    return parser.parse_args()


def ensure_local_files(repo_dir: Path) -> None:
    missing = [path for path in UPLOAD_FILES if not (repo_dir / path).exists()]
    if missing:
        raise FileNotFoundError(f"Missing required local files: {', '.join(missing)}")


def ensure_remote_dir(sftp: paramiko.SFTPClient, remote_dir: str) -> None:
    parts = [part for part in remote_dir.split("/") if part]
    current = "/"
    for part in parts:
        current = posixpath.join(current, part)
        try:
            sftp.stat(current)
        except FileNotFoundError:
            sftp.mkdir(current)


def upload_file(sftp: paramiko.SFTPClient, local_path: Path, remote_path: str) -> None:
    ensure_remote_dir(sftp, posixpath.dirname(remote_path))
    sftp.put(str(local_path), remote_path)
    local_mode = local_path.stat().st_mode
    sftp.chmod(remote_path, stat.S_IMODE(local_mode))


def exec_checked(ssh: paramiko.SSHClient, command: str) -> None:
    print(f"\n--- {command} ---", flush=True)
    stdin, stdout, stderr = ssh.exec_command(command)
    for line in stdout:
        print(line.rstrip(), flush=True)
    for line in stderr:
        print(line.rstrip(), file=sys.stderr, flush=True)
    exit_status = stdout.channel.recv_exit_status()
    if exit_status != 0:
        raise RuntimeError(f"Remote command failed with exit status {exit_status}: {command}")


def main() -> int:
    args = parse_args()
    repo_dir = Path(__file__).resolve().parent
    ensure_local_files(repo_dir)

    ssh = paramiko.SSHClient()
    ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())

    try:
        print(f"Connecting to {args.host}:{args.port} as {args.user}...", flush=True)
        ssh.connect(
            hostname=args.host,
            port=args.port,
            username=args.user,
            password=args.password,
            timeout=20,
            banner_timeout=20,
            auth_timeout=20,
            look_for_keys=False,
            allow_agent=False,
        )

        sftp = ssh.open_sftp()
        try:
            ensure_remote_dir(sftp, args.remote_dir)
            for rel_path in UPLOAD_FILES:
                local_path = repo_dir / rel_path
                remote_path = posixpath.join(args.remote_dir, rel_path.replace(os.sep, "/"))
                print(f"Uploading {rel_path} -> {remote_path}", flush=True)
                upload_file(sftp, local_path, remote_path)
        finally:
            sftp.close()

        exec_checked(
            ssh,
            "export DEBIAN_FRONTEND=noninteractive && apt-get update -y && (apt-get install -y docker.io docker-compose-plugin iptables iproute2 ca-certificates curl python3 openssl || apt-get install -y docker.io docker-compose iptables iproute2 ca-certificates curl python3 openssl)",
        )
        exec_checked(ssh, "systemctl enable docker && systemctl restart docker")
        exec_checked(
            ssh,
            f"mkdir -p {args.remote_dir}/config && test -f {args.remote_dir}/config/server.key || openssl rand 32 > {args.remote_dir}/config/server.key && chmod 600 {args.remote_dir}/config/server.key",
        )
        exec_checked(
            ssh,
            f"cd {args.remote_dir} && AIVPN_SKIP_DOWNLOAD=1 ./deploy-server-release.sh",
        )
    except (socket.error, paramiko.SSHException, RuntimeError, FileNotFoundError) as exc:
        print(f"Deploy failed: {exc}", file=sys.stderr)
        return 1
    finally:
        ssh.close()

    print("\nRemote deploy finished successfully.", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
