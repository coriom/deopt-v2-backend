# DeOpt — acceptation du Compose Raw privé

## Provenance de la preuve

L'opérateur a confirmé une nouvelle **Application Git Docker Compose**, distincte de l'ancien Service jamais déployé : dépôt public backend, `/compose.coolify-readonly.yml`, mode `Raw (deploy file as-is)`, déploiements manuels uniquement, quatre variables non secrètes et aucun domaine. Le contenu de « Preview Generated Compose » a été transmis dans la conversation. Le chemin annoncé `/tmp/deopt-coolify-raw-generated.yml` n'était pas visible dans l'environnement Codex ; aucune récupération authentifiée par l'API Coolify n'a été possible (GET local `/api/v1/applications` → 401 sans jeton). Le texte fourni a été reconstruit dans un fichier temporaire non commité. Sa reconstruction est identique octet pour octet au Compose source, SHA-256 `524bd14b0cb6d23672eabb7c793c2c649f2c760651370d33592c1a6622745f82`. Cette preuve est **un Preview attesté par l'opérateur**, pas un export indépendant de l'application Raw. L'UUID de la nouvelle application n'a pas été fourni et n'est pas déduit de l'ancien Service.

## Contrôle local

Les quatre variables `DEOPT_BACKEND_IMAGE`, `DEOPT_POSTGRES_IMAGE`, `DEOPT_CORS_ORIGIN` et `DEOPT_POSTGRES_PASSWORD_FILE` ont été résolues depuis `deploy/vps/coolify-staging-release-v1.json`. `docker compose config --format json` a réussi sur la reconstruction du Preview Raw ; `check_coolify_private_compose.py --release-manifest ...` a répondu `PRIVATE_COMPOSE=PASS; DEPLOYMENT_NOT_AUTHORIZED`.

- PostgreSQL : **`db` uniquement**, `internal: true`, volume persistant, secret par fichier, image PostgreSQL 16 au digest approuvé, aucun port hôte.
- Backend : **`db` et `web` uniquement**, image GHCR au digest approuvé, UID/GID 1001, filesystem en lecture seule, `cap_drop: ALL`, `no-new-privileges`, limites CPU/RAM, `/ready`, aucun port hôte ni route proxy.
- Aucun `env_file` dans le Preview transmis. Les drapeaux fixes imposent Base Sepolia 84532, lecture seule, indexer/trading/broadcast/worker économique désactivés. L'historique wallet non authentifié reste bloqué par le code de la release.
- Les 12 tests ciblés du validateur passent après ajout des exigences exactes `postgres={db}`, `backend={db,web}`, `networks={db,web}` et refus des `env_file` non examinés. Le rendu de l'ancien Service, avec réseau externe ajouté, reste refusé.

**Verdict du Preview fourni : `PRIVATE_COMPOSE=PASS_OPERATOR_ATTESTED_PREVIEW`.** Ce verdict n'est ni un test de runtime ni une autorisation de déploiement. Avant le premier Deploy, confirmer dans Coolify l'identité de l'Application Raw, le commit Git chargé, le mode manuel, l'absence de domaine, et répéter le contrôle si le Preview change. Ne jamais démarrer l'ancien Service.

## Port 8080 et sauvegarde

Le proxy Coolify écoute encore sur 8080 en IPv4 et IPv6 ; `API_INSECURE=false` et aucun entrypoint 8080 avaient été relevés. La correction persistante dans `Servers → localhost → Proxy → Configuration`, avec sauvegarde de l'original, diff limité aux publications 8080 et rollback, est préparée mais **ni autorisée ni appliquée**. Le test local de rollback ne vaut pas test sur le VPS. Ce changement réseau est indépendant du Compose privé DeOpt.

`deploy/vps/backup-offsite.sh` cible le conteneur PostgreSQL du projet Compose revu, chiffre le dump avant transfert et compare le SHA-256 du ciphertext téléchargé. `deploy/vps/restore-offsite-drill.sh` refuse une base non jetable ou non locale et restaure en transaction unique ; la comparaison logique après restore reste obligatoire. Le drill PostgreSQL local antérieur est réutilisé, pas présenté comme un nouvel upload/restore hors VPS. Il manque une destination hors VPS approuvée (Cloudflare R2 S3 compatible est un candidat, pas une ressource créée), un destinataire GPG et une garde indépendante de la clé, la rétention/alerte, puis un premier transfert et un restore isolé réels.

## Séquence de première release, après approbation séparée

1. Garder l'ancien Service arrêté. Confirmer l'Application Raw et son Preview inchangé ; le seul déploiement sera manuel depuis cette Application Coolify. L'image reste épinglée à `ghcr.io/coriom/deopt-v2-backend@sha256:b7ed384e9cae879579cf255fac79cce087de7f7c6922e65940c1147c78382436`. Le Compose n'a pas changé depuis le commit embarqué `e2f139fc33286fea06d94989b31ab9d33aab8b98`.
2. Choisir et tester une destination de backup DeOpt hors VPS et la récupération de la clé. Le backup interne Coolify est distinct. Préparer le projet Compose exact et le chemin de secret hôte sans afficher la valeur.
3. Après autorisation distincte, démarrer une seule fois. Vérifier volume PostgreSQL, migrations initiales et schéma, `/health`, `/ready`, absence de ports hôte/routes publiques, UID non root, secret lisible uniquement par les conteneurs prévus, charge CPU/RAM, indisponibilité honnête des marchés, historique wallet refusé, écriture/trading/broadcast refusés. Les migrations ne sont jamais rétrogradées automatiquement.
4. Exécuter un backup DeOpt chiffré vers la destination retenue, relire son ciphertext, restaurer sur une base locale jetable et comparer tables, fills, comptes et checkpoints. En cas d'échec applicatif, revenir au digest précédent seulement si le schéma reste compatible ; préserver le volume et arrêter le backend sans déclencher d'opération économique.

**DEPLOYMENT_READINESS=BLOCKED** : destination et reprise hors VPS non choisies/testées ; premier déploiement non autorisé. Le Compose Raw transmis est conforme, mais aucune preuve de runtime ou restauration hébergée n'existe. Aucune transaction blockchain, modification Vercel ou activation du trading.
