#!/usr/bin/env bash
# Operator-run, root-owned copy only. Target: the one reviewed Netcup Debian 13 VPS.
# Never accepts a command, package list, secret or remote host as an argument.
set -euo pipefail
umask 077

[[ "${1:-}" == --execute && $# == 1 && $EUID == 0 ]] || {
  echo 'Run the reviewed root-owned copy with sudo --execute in the operator terminal' >&2; exit 2;
}
[[ -t 0 ]] || { echo 'Interactive operator terminal required for SSH recovery checkpoint' >&2; exit 2; }
[[ "$(stat -c %u:%g:%a "$0")" == 0:0:700 ]] || {
  echo 'Executable must be a root-owned mode-0700 copy' >&2; exit 2;
}
[[ "$(hostname)" == v2202611448349549977 ]] || { echo 'Unexpected host' >&2; exit 2; }
source /etc/os-release
[[ "$ID" == debian && "$VERSION_ID" == 13 && "$(dpkg --print-architecture)" == amd64 ]] || {
  echo 'Expected Debian 13 amd64' >&2; exit 2;
}
ip -4 -o addr show | grep -Fq '94.16.113.122/' || { echo 'Expected Netcup IPv4 missing' >&2; exit 2; }
host_key_fingerprint=$(ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub -E sha256 | awk '{print $2}')
[[ "$host_key_fingerprint" == SHA256:cLC+iqPLwsu2CMkzZnFfaIefkljcr9mct2VR0M1mGhQ ]] || {
  echo 'SSH host key differs from the pinned WSL key; stop' >&2; exit 2;
}
printf '%s' 'Compare this host key fingerprint in the Netcup console, then type HOSTKEY_VERIFIED: '
read -r host_key_ack
[[ "$host_key_ack" == HOSTKEY_VERIFIED ]] || { echo 'Independent host-key verification pending' >&2; exit 2; }
[[ -f /root/deopt-bootstrap/prepare_coolify_vendor.py && -f /root/deopt-bootstrap/coolify-loopback.override.yml ]] || {
  echo 'Root-owned reviewed companion files missing' >&2; exit 2;
}
for item in /root/deopt-bootstrap/*.py /root/deopt-bootstrap/*.yml; do
  [[ "$(stat -c %u:%g:%a "$item")" == 0:0:600 ]] || { echo 'Companion ownership or mode changed' >&2; exit 2; }
done
exec 9>/run/lock/deopt-netcup-bootstrap.lock
flock -n 9 || { echo 'Another bootstrap is running' >&2; exit 2; }

# Refuse to overwrite an unrelated Docker/Coolify installation.
if [[ -e /data/coolify/source/.env && ! -f /var/lib/deopt-bootstrap/complete ]]; then
  echo 'Existing or partial Coolify installation: reconcile manually' >&2; exit 2
fi
if [[ -f /var/lib/deopt-bootstrap/complete ]]; then
  systemctl is-active --quiet docker
  docker inspect coolify --format '{{.State.Health.Status}}' | grep -qx healthy
  docker inspect coolify --format '{{json .HostConfig.PortBindings}}' |
    python3 -c 'import json,sys; d=json.load(sys.stdin); p=sorted((x["HostIp"],x["HostPort"]) for v in d.values() for x in v); assert p==[("127.0.0.1","6001"),("127.0.0.1","6002"),("127.0.0.1","8000")]'
  echo 'BOOTSTRAP=ALREADY_COMPLETE; verify independent SSH and private tunnel'; exit 0
fi
if dpkg-query -W -f='${Status}' docker.io podman-docker 2>/dev/null | grep -q 'install ok installed'; then
  echo 'Conflicting Docker package installed; manual review required' >&2; exit 2
fi
if id -nG deoptops | tr ' ' '\n' | grep -qx docker; then
  echo 'deoptops already has root-equivalent Docker group access' >&2; exit 2
fi

# Preserve current SSH access. The operator must have already tested a second
# independent key-based session and retained Netcup console recovery.
install -d -m 0700 /var/backups/deopt-netcup-bootstrap
if [[ ! -f /var/backups/deopt-netcup-bootstrap/sshd_config.before ]]; then
  cp -p /etc/ssh/sshd_config /var/backups/deopt-netcup-bootstrap/sshd_config.before
fi
if [[ -f /root/.ssh/authorized_keys && ! -f /var/backups/deopt-netcup-bootstrap/root_authorized_keys.before ]]; then
  cp -p /root/.ssh/authorized_keys /var/backups/deopt-netcup-bootstrap/root_authorized_keys.before
fi
if grep -Eiq '^[[:space:]]*Match[[:space:]]' /etc/ssh/sshd_config /etc/ssh/sshd_config.d/*.conf 2>/dev/null; then
  echo 'SSH Match rules need manual review' >&2; exit 2
fi
ssh_dropin=/etc/ssh/sshd_config.d/01-deopt-root-login.conf
ssh_dropin_created=false
install -d -m 0755 /etc/ssh/sshd_config.d
if [[ -e "$ssh_dropin" ]]; then
  [[ "$(cat "$ssh_dropin")" == 'PermitRootLogin prohibit-password' ]] || {
    echo 'Unexpected SSH drop-in' >&2; exit 2;
  }
else
  printf '%s\n' 'PermitRootLogin prohibit-password' > "$ssh_dropin"
  chmod 0600 "$ssh_dropin"
  ssh_dropin_created=true
fi
verify_root_ssh_policy() {
  local client_addr=$1 effective line policy_count=0
  effective=$(/usr/sbin/sshd -T -C "user=root,host=localhost,addr=$client_addr") || return 1
  while IFS= read -r line; do
    case "$line" in
      'permitrootlogin prohibit-password'|'permitrootlogin without-password') ((policy_count+=1));;
      permitrootlogin*) return 1;;
    esac
  done <<< "$effective"
  [[ $policy_count -eq 1 ]]
}
/usr/sbin/sshd -t
for client_addr in 203.0.113.5 2001:db8::5; do
  verify_root_ssh_policy "$client_addr" || {
    echo "Root password SSH still permitted or sshd -T failed for $client_addr" >&2; exit 2;
  }
done
if [[ "$ssh_dropin_created" == true ]]; then
  systemctl reload ssh
fi
echo 'SSH_ROOT_PASSWORD_LOGIN=DISABLED; DEOPTOPS_PASSWORD_SETTING_UNCHANGED'
printf '%s' 'Verify a NEW independent key-based SSH session and Netcup console recovery, then type VERIFIED: '
read -r ssh_recovery_ack
[[ "$ssh_recovery_ack" == VERIFIED ]] || { echo 'SSH recovery checkpoint not confirmed' >&2; exit 2; }

# Official Docker apt repository for Debian Trixie. Never purge existing data.
if command -v docker >/dev/null 2>&1; then
  [[ "$(dpkg-query -W -f='${Status}' docker-ce 2>/dev/null || true)" == 'install ok installed' ]] || {
    echo 'Existing Docker is not the reviewed official package' >&2; exit 2;
  }
else
  DEBIAN_FRONTEND=noninteractive apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq ca-certificates curl gnupg jq
  install -d -m 0755 /etc/apt/keyrings
  curl -fsSL --proto '=https' --tlsv1.2 https://download.docker.com/linux/debian/gpg \
    -o /etc/apt/keyrings/docker.asc
  chmod 0644 /etc/apt/keyrings/docker.asc
  docker_key=$(gpg --show-keys --with-colons /etc/apt/keyrings/docker.asc | awk -F: '$1=="fpr" {print $10; exit}')
  [[ "$docker_key" == 9DC858229FC7DD38854AE2D88D81803C0EBFCD88 ]] || {
    echo 'Docker repository signing key changed' >&2; exit 2;
  }
  if [[ -e /etc/apt/sources.list.d/docker.sources ]]; then
    echo 'Existing Docker repository definition needs review' >&2; exit 2
  fi
  cat > /etc/apt/sources.list.d/docker.sources <<'SOURCE'
Types: deb
URIs: https://download.docker.com/linux/debian
Suites: trixie
Components: stable
Architectures: amd64
Signed-By: /etc/apt/keyrings/docker.asc
SOURCE
  chmod 0644 /etc/apt/sources.list.d/docker.sources
  DEBIAN_FRONTEND=noninteractive apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
    docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
fi
systemctl enable --now docker >/dev/null
docker info >/dev/null
docker compose version >/dev/null
docker_version=$(docker version --format '{{.Server.Version}}')
[[ "$(printf '%s\n' 28.0.1 "$docker_version" | sort -V | head -1)" == 28.0.1 ]] || {
  echo 'Docker Engine below reviewed loopback-port minimum 28.0.1' >&2; exit 2;
}
compose_version=$(docker compose version --short)
compose_version=${compose_version#v}
[[ "$(printf '%s\n' 2.24.4 "$compose_version" | sort -V | head -1)" == 2.24.4 ]] || {
  echo 'Docker Compose lacks reviewed port override support' >&2; exit 2;
}
docker run --rm hello-world >/dev/null
if [[ -n "$(docker ps -q)" ]]; then
  echo 'Existing running containers require manual review before Coolify install' >&2; exit 2
fi
if [[ -f /etc/docker/daemon.json && ! -f /var/backups/deopt-netcup-bootstrap/docker_daemon.before ]]; then
  cp -p /etc/docker/daemon.json /var/backups/deopt-netcup-bootstrap/docker_daemon.before
fi
echo 'DOCKER_ENGINE=PASS; COMPOSE=PASS; TEST_CONTAINER=PASS'

# Pin the reviewed official Coolify inputs before root execution. Any CDN
# change is a hard stop requiring source review and a new committed manifest.
python3 /root/deopt-bootstrap/prepare_coolify_vendor.py
install -d -m 0700 /data/coolify/source
if [[ -e /data/coolify/source/docker-compose.custom.yml ]]; then
  cmp -s /root/deopt-bootstrap/coolify-loopback.override.yml \
    /data/coolify/source/docker-compose.custom.yml || {
      echo 'Existing Coolify network override differs' >&2; exit 2;
    }
else
  install -m 0600 /root/deopt-bootstrap/coolify-loopback.override.yml \
    /data/coolify/source/docker-compose.custom.yml
fi

# The patched, hash-verified official installer validates the rendered private
# port bindings immediately before starting any Coolify container. It creates
# a distinct root-side localhost service key; it never touches the WSL key.
unset ROOT_USERNAME ROOT_USER_EMAIL ROOT_USER_PASSWORD
HOME=/root USER=root AUTOUPDATE=false bash /root/deopt-bootstrap/vendor/install.sh 4.5.3

docker inspect coolify --format '{{.State.Health.Status}}' | grep -qx healthy
python3 - <<'PY'
import json, subprocess
ids = subprocess.check_output(['docker', 'ps', '-q'], text=True).split()
if not ids:
    raise SystemExit('no running containers after install')
containers = json.loads(subprocess.check_output(['docker', 'inspect', *ids], text=True))
bindings = []
for container in containers:
    name = container['Name'].lstrip('/')
    if name.startswith('deopt'):
        raise SystemExit('unexpected DeOpt container')
    for entries in (container['HostConfig']['PortBindings'] or {}).values():
        for entry in entries or []:
            bindings.append((name, entry['HostIp'], entry['HostPort']))
if sorted(bindings) != [('coolify','127.0.0.1','6001'),
                       ('coolify','127.0.0.1','6002'),
                       ('coolify','127.0.0.1','8000')]:
    for container in containers:
        name = container['Name'].lstrip('/')
        if name.startswith('coolify'):
            subprocess.run(['docker', 'stop', name], check=False, capture_output=True)
    raise SystemExit('unexpected published port; Coolify stopped')
print('COOLIFY_ADMIN_PORTS=LOOPBACK_IPV4_ONLY; POSTGRES_REDIS_PORTS=PRIVATE')
PY
if id -nG deoptops | tr ' ' '\n' | grep -qx docker; then
  echo 'Unexpected deoptops Docker group grant' >&2; exit 2
fi
install -d -m 0700 /var/lib/deopt-bootstrap
printf '%s\n' 'coolify=4.5.3 private-admin-ports=127.0.0.1' > /var/lib/deopt-bootstrap/complete
chmod 0600 /var/lib/deopt-bootstrap/complete
echo 'BOOTSTRAP=PASS; ADMIN_ACCOUNT=OPERATOR_PENDING; OFFSITE_BACKUP=PENDING'
