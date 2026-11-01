# DeOpt — préparation finale du staging privé Coolify

État du 2026-11-01 UTC. Aucune ressource DeOpt n'a été déployée. Le frontend Vercel et le protocole sont inchangés.

## Preuves fraîches et limites

- SSH strict vers `deopt-vps` : utilisateur `deoptops`, hôte `v2202611448349549977`, Docker 29.9.2, Compose 5.6.1. `sudo -n` est indisponible et le socket Docker refuse `deoptops` ; **le pull depuis le contexte Docker du serveur Coolify n'est pas encore prouvé**. L'opérateur rapporte que GHCR est connecté dans Coolify. Le script root contrôlé ci-dessous vérifie le pull sans lire ni afficher le fichier d'identifiants. Si Coolify emploie un autre utilisateur Docker que root, le résultat root seul ne suffit pas.
- `ss -lntH` montre 22, 80, 443 et 8080 sur IPv4 et IPv6 ; 8000/6001/6002 sont liés à `127.0.0.1`. Le contrôle opérateur antérieur de Traefik donnait `api.insecure=false`, `api.dashboard=true` et aucun entrypoint 8080. La publication 8080 est donc une dette de configuration probable, sans preuve de nécessité pour les autres applications. Un essai anonyme local du manifeste GHCR par digest a été refusé ou masqué ; cela ne remplace pas le pull authentifié du serveur.
- L'image backend privée fournie est épinglée à `ghcr.io/coriom/deopt-v2-backend@sha256:b7ed384e9cae879579cf255fac79cce087de7f7c6922e65940c1147c78382436`, commit OCI attendu `e2f139fc33286fea06d94989b31ab9d33aab8b98`. L'image officielle PostgreSQL 16 est épinglée à l'index `postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a` ; `docker buildx imagetools inspect` a indiqué `linux/amd64`, version 16.15, et un pull/inspect **local** a réussi. Ce test local ne vaut pas test sur le VPS.

## Profil et contrôle avant démarrage

`deploy/vps/coolify-staging-release-v1.json` est la feuille de release non secrète : deux digests, commit backend, origine CORS et chemin du secret hôte. `compose.coolify-readonly.yml` reste la seule source Compose. PostgreSQL a un volume dédié et un réseau interne, aucun port publié ; backend UID 1001, système de fichiers en lecture seule, capacités retirées, limites 2 Gio/2 CPU pour le backend et 1,5 Gio/1,5 CPU pour PostgreSQL, arrêt gracieux, `/health` et `/ready`. Les drapeaux interdisent le trading, l'indexation active, le broadcaster et les workers ; Base Sepolia 84532 uniquement. L'historique wallet sans authentification reste refusé. Les données de marché absentes restent indisponibles.

`check_coolify_private_compose.py --release-manifest ...` vérifie maintenant **les références exactes** des deux images, le chemin du secret et l'origine CORS, en plus des restrictions de réseau et de capacités. Le Compose source normalisé passe. [Coolify expose Source Compose et Deployable Compose après validation et sauvegarde d'un service Docker Compose Empty](https://coolify.io/docs/services/configuration/docker-compose) : le second peut ajouter des réseaux, labels ou volumes. **Aucun Compose réellement généré pour DeOpt n'a encore été reçu ni validé.** Créer un projet/environnement et sauvegarder le service sans cliquer Deploy permet cette vérification avant démarrage.

Le backend exécute automatiquement ses migrations PostgreSQL lorsqu'il démarre avec `PERSISTENCE_ENABLED=true` (`src/main.rs`). Pour la première release, inspecter les migrations exactes, disposer d'une sauvegarde restaurable et d'un plan de rollback compatible avec le schéma avant de cliquer Deploy. Le retour à un digest d'image précédent ne rétrograde jamais automatiquement la base.

## Intervention hôte préparée, non exécutée

`deploy/vps/prepare-coolify-private-staging-host.sh --prepare` exige root. Après vérification de sa copie et de son SHA-256, il tente seulement les pulls backend/PG par digest, vérifie architecture et commit OCI, imprime uniquement des booléens filtrés du proxy, puis crée si absent `/srv/deopt/staging/secrets/postgres_password` avec répertoires root `0700` et fichier root:groupe 1001 `0640`. Une valeur préexistante est préservée si ses attributs sont conformes ; tout état contradictoire arrête le script. Le secret n'est jamais affiché, transmis ou commité. Le backend UID/GID 1001 devra encore prouver sa lecture du secret au premier test de runtime autorisé. Le script n'exécute aucun `docker run`, `compose up` ni changement réseau.

Le script doit être copié dans un fichier root-owned non modifiable par `deoptops`, après contrôle de son empreinte SHA-256 `59075ec97e7ce6b8890e1e7ac10496df9c0f3abf85b5a5c7420e347ef9a27f1d`. Son exécution requiert le `sudo` interactif de l'opérateur ; Codex ne l'a pas exécuté. La création de `DeOpt/staging` dans Coolify reste aussi une action UI opérateur, sans ressource démarrée.

## Proxy 8080 : changement séparé

Ne pas éditer uniquement `/data/coolify/proxy/docker-compose.yml` : [Coolify réécrit ce fichier depuis sa configuration sauvegardée](https://coolify.io/docs/core/networking/proxy/traefik/dashboard). Après approbation distincte, exporter la configuration **sauvegardée dans Coolify**, conserver une copie protégée et son SHA-256, puis retirer uniquement les publications hôte 8080 en IPv4/IPv6 dans `Servers > localhost > Proxy > Configuration`. Conserver 80/443, les routes et les réseaux. Avant redémarrage, valider la configuration proposée ; après redémarrage, tester 80/443, chaque application existante, le tunnel privé 8000 et l'absence d'écoute 8080. En cas d'échec, restaurer la copie sauvegardée dans l'UI et redémarrer le proxy, puis recontrôler les mêmes chemins. Un test local de fixture confirme le retrait isolé de 8080 et la restitution exacte de l'état initial. **Le rollback réel sur le proxy Netcup n'a pas été testé ni appliqué.**

## Sauvegarde et décision

La sauvegarde Coolify téléchargée par l'opérateur concerne sa base interne ; elle ne couvre pas le futur volume PostgreSQL DeOpt. `deploy/vps/backup-offsite.sh` prépare `pg_dump`, chiffrement GPG et transfert rclone, mais il manque une destination hors VPS, une garde indépendante de la clé, la rétention/alerte et un restore jetable de la future base. Aucun backup DeOpt, transfert ou restore hébergé n'est revendiqué.

**PRIVATE_RELEASE=BLOCKED** jusqu'à : (1) pull dans le contexte Docker réellement utilisé par Coolify et commit OCI vérifiés ; (2) secret et projet `DeOpt/staging` préparés ; (3) Deployable Compose généré contrôlé sans port ni route publique ; (4) sauvegarde DeOpt hors VPS et restauration jetable ; (5) revue des migrations et autorisation distincte de release. Le port 8080 du proxy est un correctif réseau séparé avec retour arrière ; il ne doit pas être modifié implicitement par la release DeOpt. Aucun signer, broadcaster, transaction publique ou trading n'est autorisé.

## Suite opérateur, sans déploiement

Validation locale fraîche : 22 tests ciblés PASS ; `docker compose config` et le contrôle `--release-manifest` du Compose source PASS ; syntaxe Bash, JSON et `git diff --check` PASS. Le Compose généré par Coolify, le pull authentifié hôte et la restauration DeOpt demeurent non vérifiés.

1. Dans la session Coolify privée, confirmer que le serveur `localhost` utilise le contexte Docker root avant le contrôle de pull. L'opérateur peut ensuite exécuter **une seule intervention sudo** avec la copie root-owned du script hôte, dont le SHA-256 exact figure dans le rapport final.
2. Créer uniquement le projet `DeOpt` et l'environnement `staging`, puis un service `Docker Compose Empty` à partir de `compose.coolify-readonly.yml` ; définir les quatre valeurs non secrètes du manifeste de release (digests, CORS, chemin du fichier secret), sans domaine ni auto-déploiement. Sauvegarder et lire `Deployable Compose` ; **ne pas cliquer Deploy**.
3. Copier le Deployable Compose dans un fichier privé local, normaliser par `docker compose config --format json`, puis exécuter `python3 deploy/vps/check_coolify_private_compose.py --release-manifest deploy/vps/coolify-staging-release-v1.json rendu.json`. Ne pas commiter ni transmettre le rendu s'il contient des valeurs sensibles. Un PASS source seul ne remplace pas ce contrôle.
4. Choisir et éprouver la destination de sauvegarde DeOpt. Revenir ensuite pour une autorisation de release privée séparée, avec le digest et le Compose généré validés.
