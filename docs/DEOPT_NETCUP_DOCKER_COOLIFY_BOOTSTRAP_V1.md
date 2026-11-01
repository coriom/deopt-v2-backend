# Netcup Debian 13: Docker et Coolify, point de reprise opérateur

**Mise à jour du 2026-11-01 12:54 UTC :** Docker 29.9.2, Compose 5.6.1 et SSH sont actifs sur le VPS. `/data` est absent, seuls les ports SSH écoutent ; l'inventaire Docker des conteneurs exige root et reste non vérifié par `deoptops`. La préparation Coolify s'est arrêtée avant son installation : `urllib` a reçu HTTP 403 sur `upgrade-postgres.sh`, tandis que `curl` a reçu HTTP 200 sans redirection pour ce fichier et son SHA-256 correspond au hash approuvé. La procédure de reprise et ses nouvelles empreintes figurent à la fin de ce document ; la copie intégrale ci-dessous décrit le bootstrap initial et n'est plus nécessaire.

Vérifié le 2026-11-01 UTC, **sans modification du VPS**. Cible unique : `deoptops@94.16.113.122`, hostname `v2202611448349549977`, Debian 13 amd64. L'agent SSH keychain existant a fourni la clé `SHA256:4+ZAflWevkavw030+70CA8xDzkSSNqK7gfoAhEIN6KA`. La clé d'hôte ED25519 connue par WSL est `SHA256:cLC+iqPLwsu2CMkzZnFfaIefkljcr9mct2VR0M1mGhQ` ; **sa comparaison indépendante depuis la console Netcup reste à faire**. Deux connexions strictes, indépendantes et sans agent forwarding ont réussi. `sudo -n` échoue ; **seul l'opérateur peut lancer le bootstrap avec sudo interactif**. Ne jamais communiquer son mot de passe ni la phrase secrète de la clé.

État mesuré : 4 vCPU ; 7,8 Gio de RAM (7,4 Gio disponibles lors du relevé) ; `/` sur `/dev/vda4`, 157 Gio, 149 Gio libres ; SSH actif sur `0.0.0.0:22` et `[::]:22`. Docker, Compose et Coolify absents au contrôle. Les conteneurs étaient inaccessibles parce que Docker est absent, pas prouvés « arrêtés ». Les lignes SSH visibles indiquaient `PermitRootLogin yes` et `PasswordAuthentication yes`; le bootstrap réduit l'accès root à la clé sans supprimer l'accès initial de `deoptops`. Aucun pare-feu utilisateur n'était visible ; les règles effectives privilégiées et l'exposition externe après installation restent à vérifier.

## Copie exacte et exécution regroupée

Fichiers à transférer dans `~/deopt-bootstrap.stage/` (aucun secret) :

Le transfert SFTP/SSH non privilégié a été réalisé le 2026-11-01 ; le dossier distant appartient à `deoptops`, mode `0700`. Le 2026-11-01, le seul script corrigé a été retransféré et les trois SHA-256 distants ont été revérifiés. Aucun fichier système root n'a été modifié par ce transfert.

| Fichier | SHA-256 attendu |
|---|---|
| `netcup-docker-coolify-bootstrap.sh` | `68848fed094e8da65b1214c59bd3c9970aa780ef3f78e58245ba19f3b7c82d2b` |
| `prepare_coolify_vendor.py` | `160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95` |
| `coolify-loopback.override.yml` | `0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856` |

Les empreintes doivent être **comparées automatiquement dans le terminal opérateur** avant puis après la copie root-owned. La copie dans `/root/deopt-bootstrap/` doit appartenir à root, mode `0700` pour le script et `0600` pour ses deux dépendances. Le script refuse toute autre propriété ou mode. La préparation distante user-owned n'accorde aucun privilège ; elle est seulement l'entrée de la copie vérifiée. Le contrôle `sshd -T` accepte exactement `prohibit-password` ou son synonyme OpenSSH `without-password` ; les autres valeurs, une sortie vide et une commande en échec restent refusées. La vérification `sshd -t` reste obligatoire.

Dans **le terminal SSH de l'opérateur** (après contrôle des trois empreintes ci-dessus) :

```bash
set -euo pipefail
cat <<'SHA' | sha256sum --check --status
68848fed094e8da65b1214c59bd3c9970aa780ef3f78e58245ba19f3b7c82d2b  /home/deoptops/deopt-bootstrap.stage/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /home/deoptops/deopt-bootstrap.stage/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /home/deoptops/deopt-bootstrap.stage/coolify-loopback.override.yml
SHA
sudo install -d -o root -g root -m 0700 /root/deopt-bootstrap
sudo install -o root -g root -m 0700 ~/deopt-bootstrap.stage/netcup-docker-coolify-bootstrap.sh /root/deopt-bootstrap/
sudo install -o root -g root -m 0600 ~/deopt-bootstrap.stage/prepare_coolify_vendor.py /root/deopt-bootstrap/
sudo install -o root -g root -m 0600 ~/deopt-bootstrap.stage/coolify-loopback.override.yml /root/deopt-bootstrap/
cat <<'SHA' | sudo sha256sum --check --status
68848fed094e8da65b1214c59bd3c9970aa780ef3f78e58245ba19f3b7c82d2b  /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh
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

Un seul fichier non secret, `prepare_coolify_vendor.py`, a changé. Dans le terminal SSH de l'opérateur, copier **seulement** ce fichier depuis le staging dans le répertoire root-owned existant, avec arrêt sur toute erreur :

```bash
set -euo pipefail
cat <<'SHA' | sha256sum --check --status
68848fed094e8da65b1214c59bd3c9970aa780ef3f78e58245ba19f3b7c82d2b  /home/deoptops/deopt-bootstrap.stage/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /home/deoptops/deopt-bootstrap.stage/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /home/deoptops/deopt-bootstrap.stage/coolify-loopback.override.yml
SHA
sudo install -o root -g root -m 0600 \
  /home/deoptops/deopt-bootstrap.stage/prepare_coolify_vendor.py \
  /root/deopt-bootstrap/prepare_coolify_vendor.py
cat <<'SHA' | sudo sha256sum --check --status
68848fed094e8da65b1214c59bd3c9970aa780ef3f78e58245ba19f3b7c82d2b  /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh
160e174c8b4c337b94819377ea6a2cb9b0099705b646bb8b5e0d4c5d3ebe0b95  /root/deopt-bootstrap/prepare_coolify_vendor.py
0df01f752a0959b095738f741bf9f632d7334f0dac914b0ed850ea315cbe9856  /root/deopt-bootstrap/coolify-loopback.override.yml
SHA
echo 'COPIE_ROOT_OWNED=VERIFIED'
```

Puis lancer séparément `sudo /root/deopt-bootstrap/netcup-docker-coolify-bootstrap.sh --execute`. Le script demande de nouveau `HOSTKEY_VERIFIED` après comparaison indépendante depuis la console Netcup, puis `VERIFIED` après une nouvelle connexion SSH indépendante. Il revérifie Docker, les empreintes Coolify et les ports privés avant le premier démarrage. Un éventuel état Coolify partiel inattendu reste un arrêt obligatoire ; ne pas supprimer `/data` pour contourner ce contrôle.
