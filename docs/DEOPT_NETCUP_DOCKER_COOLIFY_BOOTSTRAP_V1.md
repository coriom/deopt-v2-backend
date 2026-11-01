# Netcup Debian 13: Docker et Coolify, point de reprise opérateur

**Reprise SSH du 2026-11-01 13:30 UTC :** le bootstrap échouait sur son pipeline `sshd -T | grep -q` malgré un résultat IPv4 root `without-password` rapporté par l'opérateur. La correction et l'unique bloc opérateur à utiliser figurent dans la dernière section. Les sections précédentes conservent le contexte historique.

**Mise à jour du 2026-11-01 12:54 UTC :** Docker 29.9.2, Compose 5.6.1 et SSH sont actifs sur le VPS. `/data` est absent, seuls les ports SSH écoutent ; l'inventaire Docker des conteneurs exige root et reste non vérifié par `deoptops`. La préparation Coolify s'est arrêtée avant son installation : `urllib` a reçu HTTP 403 sur `upgrade-postgres.sh`, tandis que `curl` a reçu HTTP 200 sans redirection pour ce fichier et son SHA-256 correspond au hash approuvé. La procédure de reprise et ses nouvelles empreintes figurent à la fin de ce document ; la copie intégrale ci-dessous décrit le bootstrap initial et n'est plus nécessaire.

Vérifié le 2026-11-01 UTC, **sans modification du VPS**. Cible unique : `deoptops@94.16.113.122`, hostname `v2202611448349549977`, Debian 13 amd64. L'agent SSH keychain existant a fourni la clé `SHA256:4+ZAflWevkavw030+70CA8xDzkSSNqK7gfoAhEIN6KA`. La clé d'hôte ED25519 connue par WSL est `SHA256:cLC+iqPLwsu2CMkzZnFfaIefkljcr9mct2VR0M1mGhQ` ; **sa comparaison indépendante depuis la console Netcup reste à faire**. Deux connexions strictes, indépendantes et sans agent forwarding ont réussi. `sudo -n` échoue ; **seul l'opérateur peut lancer le bootstrap avec sudo interactif**. Ne jamais communiquer son mot de passe ni la phrase secrète de la clé.

État mesuré : 4 vCPU ; 7,8 Gio de RAM (7,4 Gio disponibles lors du relevé) ; `/` sur `/dev/vda4`, 157 Gio, 149 Gio libres ; SSH actif sur `0.0.0.0:22` et `[::]:22`. Docker, Compose et Coolify absents au contrôle. Les conteneurs étaient inaccessibles parce que Docker est absent, pas prouvés « arrêtés ». Les lignes SSH visibles indiquaient `PermitRootLogin yes` et `PasswordAuthentication yes`; le bootstrap réduit l'accès root à la clé sans supprimer l'accès initial de `deoptops`. Aucun pare-feu utilisateur n'était visible ; les règles effectives privilégiées et l'exposition externe après installation restent à vérifier.

## Copie exacte et exécution regroupée

Fichiers à transférer dans `~/deopt-bootstrap.stage/` (aucun secret) :

Le transfert SFTP/SSH non privilégié a été réalisé le 2026-11-01 ; le dossier distant appartient à `deoptops`, mode `0700`. Le 2026-11-01, le seul script corrigé a été retransféré et les trois SHA-256 distants ont été revérifiés. Aucun fichier système root n'a été modifié par ce transfert.

| Fichier | SHA-256 attendu |
|---|---|
| `netcup-docker-coolify-bootstrap.sh` | `4061a4dd4c89c56fdb8d61542a4d6d26963b2ccc5d8b68125d24b7f2ed2f5ffb` |
| `prepare_coolify_vendor.py` | `160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95` |
| `coolify-loopback.override.yml` | `0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856` |

Les empreintes doivent être **comparées automatiquement dans le terminal opérateur** avant puis après la copie root-owned. La copie dans `/root/deopt-bootstrap/` doit appartenir à root, mode `0700` pour le script et `0600` pour ses deux dépendances. Le script refuse toute autre propriété ou mode. La préparation distante user-owned n'accorde aucun privilège ; elle est seulement l'entrée de la copie vérifiée. Le contrôle `sshd -T` accepte exactement `prohibit-password` ou son synonyme OpenSSH `without-password` ; les autres valeurs, une sortie vide et une commande en échec restent refusées. La vérification `sshd -t` reste obligatoire.

Dans **le terminal SSH de l'opérateur** (après contrôle des trois empreintes ci-dessus) :

```bash
set -euo pipefail
cat <<'SHA' | sha256sum --check --status
4061a4dd4c89c56fdb8d61542a4d6d26963b2ccc5d8b68125d24b7f2ed2f5ffb  /home/deoptops/deopt-bootstrap.stage/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /home/deoptops/deopt-bootstrap.stage/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /home/deoptops/deopt-bootstrap.stage/coolify-loopback.override.yml
SHA
sudo install -d -o root -g root -m 0700 /root/deopt-bootstrap
sudo install -o root -g root -m 0700 ~/deopt-bootstrap.stage/netcup-docker-coolify-bootstrap.sh /root/deopt-bootstrap/
sudo install -o root -g root -m 0600 ~/deopt-bootstrap.stage/prepare_coolify_vendor.py /root/deopt-bootstrap/
sudo install -o root -g root -m 0600 ~/deopt-bootstrap.stage/coolify-loopback.override.yml /root/deopt-bootstrap/
cat <<'SHA' | sudo sha256sum --check --status
4061a4dd4c89c56fdb8d61542a4d6d26963b2ccc5d8b68125d24b7f2ed2f5ffb  /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /root/deopt-bootstrap/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /root/deopt-bootstrap/coolify-loopback.override.yml
SHA
echo 'COPIE_ROOT_OWNED=VERIFIED'
```

L'exécution est une **commande séparée**, seulement après la vérification :

```sh
sudo /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh --execute
```

Gardez ce terminal et la console Netcup ouverts. Avant la première modification, le script attend `HOSTKEY_VERIFIED` : exécutez **depuis la console Netcup** `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub -E sha256` et comparez son empreinte à `SHA256:cLC+iqPLwsu2CMkzZnFfaIefkljcr9mct2VR0M1mGhQ`. Après le rechargement SSH, le script attend `VERIFIED` : ouvrez alors **une nouvelle session indépendante** depuis WSL avec l'agent keychain et vérifiez le login par clé avant de saisir ce mot de confirmation. En cas d'échec, ne fermez pas la session initiale ; utilisez la console Netcup. Le retour arrière SSH borné, si nécessaire et seulement depuis une session encore fonctionnelle ou la console, consiste à retirer `/etc/ssh/sshd_config.d/01-deopt-root-login.conf`, exécuter `/usr/sbin/sshd -t`, puis recharger `ssh`. Cela rétablit potentiellement l'ancien accès root par mot de passe : corrigez alors l'accès par clé avant de réappliquer le durcissement.

Le script vérifie hostname, adresse IPv4, OS, architecture, propriété des fichiers et absence d'installation Coolify préexistante. Il sauvegarde la configuration SSH, les clés publiques root autorisées si présentes et la configuration Docker existante si présente ; il installe un seul drop-in `PermitRootLogin prohibit-password`, valide syntaxe et paramètres effectifs IPv4/IPv6, puis recharge SSH. Il n'interdit pas le mot de passe à `deoptops` tant que la récupération n'est pas validée. Il ne change ni port ni utilisateur SSH et ne place pas `deoptops` dans le groupe `docker`.

Ensuite il installe Docker Engine, CLI, containerd, Buildx et Compose depuis le dépôt APT officiel Debian Trixie, après vérification de l'empreinte de sa clé. Il refuse Docker inférieur à **28.0.1** (les anciennes versions avaient une faiblesse de publication de ports loopback) et Compose inférieur à **2.24.4**. Il démarre Docker et teste `hello-world`. Il n'effectue ni upgrade majeur de Debian, ni purge, ni reboot. L'installateur Coolify peut régler les logs et les pools d'adresses Docker, et installer `curl`, `wget`, `git`, `jq`, `openssl` ; le serveur est vierge, et le script refuse tout conteneur déjà actif avant l'installation.

## Protection Coolify avant le premier démarrage

La procédure officielle Coolify publie par défaut `8000`, `6001`, `6002` sur toutes les interfaces ; sa page d'inscription initiale donne le contrôle du serveur au premier inscrit. Pour éviter cette exposition, notre script télécharge les **sept entrées officielles vérifiées par SHA-256** (installateur, deux Compose, environnement modèle, upgrade, upgrade PostgreSQL et versions), les épingle dans `/root/deopt-bootstrap/vendor`, et remplace uniquement leur base CDN par cette copie root-owned. La version préparée est **Coolify 4.5.3**, helper **1.0.19**. Si un fichier officiel a changé, le script s'arrête et exige une nouvelle revue : ne retirez pas le contrôle d'empreinte. L'installateur officiel ainsi borné vérifie le Compose rendu **avant** tout démarrage de conteneur.

L'override `deploy/vps/coolify-loopback.override.yml` remplace explicitement les trois publications par `127.0.0.1:8000`, `127.0.0.1:6001`, `127.0.0.1:6002`. La validation Compose locale a confirmé la fusion, sans publication IPv6. Après installation, une vérification indépendante des bindings Docker stoppe les conteneurs Coolify si un port public inattendu apparaît. Aucun domaine ni proxy public 80/443 n'est créé ici. L'accès technique SSH de Coolify est une **clé distincte générée sur le VPS**, autorisée pour la gestion localhost par l'installateur ; le logiciel a donc un pouvoir équivalent à root via Docker et cette clé. La clé personnelle WSL reste chiffrée et ne quitte pas WSL. Les mises à jour automatiques Coolify sont désactivées pour éviter une modification non revue des ports ; les futures mises à jour devront conserver l'override et être vérifiées.

Après `BOOTSTRAP=PASS`, le propriétaire ouvre depuis WSL :

```sh
ssh -o StrictHostKeyChecking=yes -o ForwardAgent=no -o ExitOnForwardFailure=yes \
  -L 127.0.0.1:8000:127.0.0.1:8000 \
  -L 127.0.0.1:6001:127.0.0.1:6001 \
  -L 127.0.0.1:6002:127.0.0.1:6002 -N deopt-vps
```

Ouvrir **`http://127.0.0.1:8000`** dans son navigateur, créer le premier compte propriétaire et activer la 2FA. Ne communiquer aucun mot de passe dans le chat. Vérifier ensuite dans Coolify `Servers > localhost > General` que le serveur est joignable, sans créer d'application DeOpt. Le tunnel et un conteneur healthy prouvent des choses distinctes : l'existence du compte, la 2FA et la possibilité de restaurer une sauvegarde exigent leurs propres contrôles.

## DeOpt et sauvegarde : préparation seulement

`compose.coolify-readonly.yml` conserve à l'identique les services backend/PostgreSQL du profil VPS validé et retire Caddy : **Coolify est le seul futur proxy**. Aucun port backend ou PostgreSQL n'est publié. Le réseau `db` reste interne, volume PostgreSQL persistant, limites CPU/RAM, health checks, arrêt contrôlé, utilisateur backend `1001:1001`, root FS read-only et secret fichier sont préservés. Chaîne 84532 uniquement ; trading, execution, broadcaster, workers, WebSocket et lectures marché restent désactivés ; l'historique wallet reste HTTP 403 jusqu'à authentification. Ce fichier n'est **pas déployé**. Avant tout futur déploiement, examiner le Compose *généré* par Coolify, ses réseaux/ports, la propriété du secret, et la séparation de sa base interne `coolify-db` et de la base DeOpt. N'utiliser qu'une seule autorité de déploiement : ne pas lancer aussi `deploy-readonly.sh`. GHCR privé, domaine API, RPC, capacité et sauvegarde restent des portes séparées.

Coolify `Settings > Backup` doit sauvegarder **l'instance Coolify**, distinctement de la future base DeOpt. Le dump Coolify seul est insuffisant : conserver hors serveur et séparément sa clé `APP_KEY` de `/data/coolify/source/.env`, la clé de service SSH, l'override et la version, sans les afficher dans le chat ni les commettre. Choisir une destination hors VPS, vérifier l'upload et une restauration jetable avant de déclarer `OFFSITE_RECOVERY=PASS`. Aucun bucket ou stockage distant n'a été créé ici.

## Contrôles de reprise

Après l'intervention opérateur, refaire deux connexions SSH indépendantes, `/usr/sbin/sshd -t`, paramètres effectifs root, `docker version`, `docker compose version`, `docker inspect` des bindings, `ss -lnt` IPv4/IPv6, inspection de `DOCKER-USER` et du pare-feu fournisseur, état Coolify et ressources au repos. Tester que l'interface d'inscription **n'est pas accessible directement** depuis l'adresse publique, tout en restant accessible par tunnel. Vérifier qu'aucun conteneur DeOpt, worker ou base DeOpt ne tourne. Ne pas interpréter l'absence de règles UFW comme une preuve de filtrage ; le binding Docker doit être vérifié directement.

Sources officielles revues : [Docker Debian](https://docs.docker.com/engine/install/debian/), [Docker post-install](https://docs.docker.com/engine/install/linux-postinstall/), [Coolify installation](https://coolify.io/docs/start-with-self-hosted), [Coolify SSH](https://coolify.io/docs/core/infrastructure/servers/openssh), [Coolify firewall](https://coolify.io/docs/core/infrastructure/servers/firewall), [Coolify instance backup](https://coolify.io/docs/core/backup-and-recovery/instance-backup). Lors du rapport initial, le script n'avait pas encore été exécuté ; l'état actuel de Docker et Coolify figure dans la mise à jour ci-dessus.

## Reprise ciblée après le HTTP 403

Le script de préparation utilise maintenant `curl` avec HTTPS et TLS obligatoires, redirections limitées à HTTPS, délai et tentatives bornés, limite de taille et vérification des sept SHA-256 inchangés. `--fail` refuse les réponses HTTP en erreur ; un téléchargement incomplet ou un hash différent arrête le bootstrap. Le fichier bloqué a été vérifié directement depuis le VPS avec `curl` : SHA-256 `aa65c277fc13ab5b27c349fbfef12defc59c084b244e4b258329e1f6c570ae8d`. Aucun miroir GitHub n'est nécessaire. Les paquets Docker déjà installés sont réutilisés par le script ; pas de purge ni de réinstallation.

Validation locale fraîche : 10 tests ciblés PASS (dont HTTP 403, succès, hash différent, téléchargement tronqué ou interrompu et redirection HTTP refusée), syntaxe Python/Bash PASS et téléchargement réel des sept fichiers officiels avec hash et version épinglés PASS dans un répertoire temporaire. Le premier essai local non privilégié avait échoué sur DNS bloqué par le sandbox (`curl 6`) ; le même essai avec accès réseau autorisé a réussi. Aucun installateur n'a été exécuté par ce test.

Validation distante non privilégiée : le helper corrigé a été retransféré seul dans `~/deopt-bootstrap.stage/` ; ses trois empreintes de staging correspondent au tableau. La fonction `prepare()` du fichier stagé a téléchargé et vérifié les sept fichiers officiels dans un répertoire temporaire du VPS, puis ce répertoire a été supprimé automatiquement. Résultat `VPS_VENDOR_FETCH_AND_HASH=PASS count=7`. Aucun fichier sous `/root`, service ou conteneur n'a été modifié par ce test.

Au jalon HTTP 403, seul le fichier non secret `prepare_coolify_vendor.py` avait changé. La correction SSH ultérieure utilise la dernière section ci-dessous ; cette procédure est conservée comme historique.

```bash
set -euo pipefail
cat <<'SHA' | sha256sum --check --status
4061a4dd4c89c56fdb8d61542a4d6d26963b2ccc5d8b68125d24b7f2ed2f5ffb  /home/deoptops/deopt-bootstrap.stage/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /home/deoptops/deopt-bootstrap.stage/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /home/deoptops/deopt-bootstrap.stage/coolify-loopback.override.yml
SHA
sudo install -o root -g root -m 0600 \
  /home/deoptops/deopt-bootstrap.stage/prepare_coolify_vendor.py \
  /root/deopt-bootstrap/prepare_coolify_vendor.py
cat <<'SHA' | sudo sha256sum --check --status
4061a4dd4c89c56fdb8d61542a4d6d26963b2ccc5d8b68125d24b7f2ed2f5ffb  /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /root/deopt-bootstrap/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /root/deopt-bootstrap/coolify-loopback.override.yml
SHA
echo 'COPIE_ROOT_OWNED=VERIFIED'
```

Puis lancer séparément `sudo /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh --execute`. Le script demande de nouveau `HOSTKEY_VERIFIED` après comparaison indépendante depuis la console Netcup, puis `VERIFIED` après une nouvelle connexion SSH indépendante. Il revérifie Docker, les empreintes Coolify et les ports privés avant le premier démarrage. Un éventuel état Coolify partiel inattendu reste un arrêt obligatoire ; ne pas supprimer `/data` pour contourner ce contrôle.

## Reprise ciblée : politique SSH effective IPv4 et IPv6

Le script root-owned reste illisible à `deoptops` ; la copie de staging était identique au commit précédent (`68848fed…`). Sans sudo, `sshd -t` et `sshd -T` échouent sur le drop-in root mode `0600` : cela ne prouve pas une politique permissive. L'opérateur a relevé `without-password` pour root IPv4 ; la valeur IPv6 n'est pas encore attestée. Le pipeline précédent utilisait `grep -q` sous `pipefail`, donc un `SIGPIPE` de `sshd` est une cause plausible du faux échec, sans preuve suffisante pour exclure une divergence IPv6 sur l'hôte.

Le contrôle corrigé capture d'abord **toute** la sortie et le statut de `sshd -T`, puis exige une unique ligne exacte `permitrootlogin prohibit-password` ou `permitrootlogin without-password` pour chacune des deux familles. Il refuse `yes`, une ligne inconnue, une sortie vide, des lignes contradictoires et tout code d'erreur. `sshd -t` reste obligatoire. Quand le drop-in attendu existe déjà, le bootstrap ne le réécrit pas et ne recharge pas SSH ; le point de confirmation `VERIFIED` est conservé. Les tests de sorties longues sous `pipefail` et d'erreur `sshd` passent pour IPv4 et IPv6. Docker installé reste réutilisé.

**Procédure opérateur actuelle, en un bloc, dans son terminal SSH interactif** : elle vérifie d'abord la politique effective et la copie root précédente. Toute erreur arrête avant la nouvelle copie ou l'exécution. Ne pas retirer les gardes si une valeur est `yes` ; corriger alors la configuration SSH depuis une session sûre ou la console Netcup.

```bash
set -euo pipefail
sudo /usr/sbin/sshd -t
for addr in 203.0.113.5 2001:db8::5; do
  effective=$(sudo /usr/sbin/sshd -T -C "user=root,host=localhost,addr=$addr")
  policy=$(printf '%s\n' "$effective" | sed -n '/^permitrootlogin /p')
  case "$policy" in
    'permitrootlogin prohibit-password'|'permitrootlogin without-password')
      printf 'ROOT_SSH_POLICY=%s PASS\n' "$addr" ;;
    *) printf 'ROOT_SSH_POLICY=%s FAIL\n' "$addr" >&2; exit 2 ;;
  esac
done
cat <<'SHA' | sha256sum --check --status
4061a4dd4c89c56fdb8d61542a4d6d26963b2ccc5d8b68125d24b7f2ed2f5ffb  /home/deoptops/deopt-bootstrap.stage/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /home/deoptops/deopt-bootstrap.stage/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /home/deoptops/deopt-bootstrap.stage/coolify-loopback.override.yml
SHA
cat <<'SHA' | sudo sha256sum --check --status
68848fed094e8da65b1214c59bd3c9970aa780ef3f78e58245ba19f3b7c82d2b  /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /root/deopt-bootstrap/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /root/deopt-bootstrap/coolify-loopback.override.yml
SHA
sudo install -o root -g root -m 0700 \
  /home/deoptops/deopt-bootstrap.stage/netcup-docker-coolify-bootstrap.sh \
  /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh
cat <<'SHA' | sudo sha256sum --check --status
4061a4dd4c89c56fdb8d61542a4d6d26963b2ccc5d8b68125d24b7f2ed2f5ffb  /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /root/deopt-bootstrap/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /root/deopt-bootstrap/coolify-loopback.override.yml
SHA
sudo /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh --execute
```

Le bootstrap demande `HOSTKEY_VERIFIED` après comparaison depuis la console Netcup, puis `VERIFIED` après une nouvelle connexion SSH indépendante. Aucun compte Coolify ni service DeOpt n'est créé par les seuls contrôles préalables.
