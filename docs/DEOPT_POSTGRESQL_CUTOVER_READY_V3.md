# DEOPT_POSTGRESQL_CUTOVER_READY_V3

Préparation locale seulement. Aucun conteneur de production Netcup, volume actif,
proxy partagé ou schéma de production n'a été modifié. Aucun push ou déploiement
n'est autorisé par ce document.

## Périmètre prouvé le 4 novembre 2026

| Porte | Résultat et preuve |
|---|---|
| Sauvegarde réelle | Ciphertext R2 récupéré, 48 513 octets, SHA-256 `9f1c4243acb3256d98874e56649d0992caa4045c58d3f28fccbba992c46732c8`; déchiffrement par la clé privée locale WSL; restauration dans `deopt-production-restore-drill`, conteneur PostgreSQL 16 sans réseau ni port. État initial : 71 migrations, 93 tables publiques. |
| Migration 0072 | `deopt-v2-backend --migrate-only` exécuté avec `deopt_migrator` sur deux copies réelles : 72 migrations, 96 tables publiques. La seconde a suivi l'ordre exact rôles → propriété → migration → droits runtime. Rerun du migrateur via SCRAM/socket sur base synthétique : PASS. |
| Rôles | Sur copie réelle : runtime, migrator et backup sans SUPERUSER, CREATEDB, CREATEROLE, REPLICATION ni BYPASSRLS. 96 tables, 8 séquences et 8 fonctions transférées au migrateur. Le migrateur peut `ALTER` une ancienne table dans une transaction annulée ; le runtime ne le peut pas. Le runtime ne possède aucune table, n'a pas CREATE sur public, ne peut pas écrire `_sqlx_migrations`, et ne peut pas lire `fee_events`. |
| Auth API sur copie réelle | Avec runtime limité : `/health=200`, `/ready=200`, challenge wallet=200, signature invalide=401, historique wallet anonyme=401. Le test PostgreSQL complet de signature valide, anti-rejeu, concurrence sur nonce, isolation entre deux wallets, expiration, révocation et reprise de session a réussi sur copie réelle **et** via SCRAM/socket synthétique. Il a identifié le besoin précis de `SELECT` sur `indexed_perp_trades` pour `My Fees`, corrigé puis retesté. Aucun secret ou donnée wallet n'a été imprimé. |
| Sauvegarde dédiée | `pg_dump` réel de la copie restaurée avec `deopt_backup` : PASS. Sur base synthétique, `pg_dump` via SCRAM/socket : PASS, DDL refusé. L'upload R2 avec le nouveau rôle reste à valider après le basculement autorisé. |
| Topologie Docker locale | PostgreSQL `network_mode=none`, aucun port ; backend réseau `web` seul, port 8080 interne seulement ; socket Unix partagé en lecture seule côté backend ; `/ready=200`, historique anonyme=401. La recréation du backend a conservé le volume PGDATA. |
| Repli local | Image locale de repli construite depuis l'image candidate compatible schéma 72 ; son entrypoint force `WALLET_READ_AUTH_ENABLED=false`. Sur le **volume PostgreSQL réel restauré**, le script `test_real_restore_rollback_v3.sh` a exécuté image candidate → image de repli → image candidate via SCRAM/socket, sans réseau ni port : `/ready=200`, historique anonyme 401 → 403 → 401, 72 migrations et PGDATA conservé. La topologie Compose synthétique a aussi passé ces transitions. |
| Données historiques | Seconde restauration opérateur de la même sauvegarde : 71 migrations et 93 tables avant, 72 migrations et 96 tables après. Les blocs `COPY` des tables historiques et les valeurs des séquences, hors journal de migrations et nouvelles tables wallet, donnent le même SHA-256 avant/après : PASS. Le premier comparateur incluait les commentaires `pg_dump` dont le propriétaire change ; son FAIL a été diagnostiqué puis corrigé sans afficher de ligne. |

La sauvegarde automatique R2 du 4 novembre a été rapportée `success`, avec
trois objets chiffrés et un dernier objet de 48 513 octets. C'est une preuve
de la configuration en cours, pas une preuve que le futur rôle backup fonctionne
déjà sur le VPS. La restauration isolée précédente a réussi ; la présente
restauration avec la sauvegarde la plus récente a aussi réussi.

Le rôle existant `deopt_readonly` reste superutilisateur administrateur avec
son secret hôte protégé, mais il n'est utilisé ni par le futur backend runtime,
ni par la future tâche de sauvegarde. Le renommer ou le rendre `NOLOGIN`
demanderait un mécanisme d'administration distinct et n'est pas inclus dans ce
basculement. L'API n'obtient que le secret `deopt_runtime`.

## Fichiers de release

- `compose.coolify-least-privilege-readonly.yml` : candidat Raw, PostgreSQL
  sans réseau Docker, backend sur `web`, trois secrets distincts et flags
  économiques inactifs.
- `deploy/vps/postgres-least-privilege-{pre,ownership,post}.sql` : préparation
  des rôles, transfert de propriété et droits après migration.
- `deploy/vps/operator-prepare-least-privilege-v3.sh` : **à exécuter seulement
  après une autorisation de basculement**, depuis une copie root-owned des SQL.
  Crée des secrets distincts sans les afficher ; vérifie l'image, le volume,
  les 71 migrations et l'absence de journalisation SQL des identifiants.
- `deploy/vps/operator-migrate-least-privilege-v3.sh` : exécution contrôlée de
  0072 dans un conteneur jetable sans réseau, avec rôle migrateur et image par
  digest, puis droits runtime. Ne s'exécute qu'après activation du socket.
- `deploy/vps/backup-offsite-least-privilege.sh` : `pg_dump` par le rôle backup,
  chiffrement GPG avant transfert R2 et comparaison du ciphertext distant.
- `deploy/vps/run-daily-production-backup-least-privilege.sh` : même verrou,
  état de succès et garde d'intégrité que le timer actuel, mais référence le
  script de dump par le rôle backup. Installer les deux en copies root-owned
  après vérification SHA-256 lors du basculement, puis remplacer la cible
  `/usr/local/sbin/deopt-production-backup` ; ne pas modifier le timer.
- `Dockerfile.cutover-fallback` : construit une image de repli depuis le digest
  de la nouvelle image compatible schéma 72. L'ancien digest de production
  (source `e2f139fc…`) ne peut pas servir de repli après 0072.
- `deploy/vps/coolify-least-privilege-release-v3.json` : manifeste de candidat.
  Les digests backend/repli restent `null` et rendent tout déploiement prématuré
  impossible.

## Séquence de basculement à autoriser séparément

1. Préflight Coolify Raw, configuration réelle et volume PGDATA ; sauvegarde
   R2 chiffrée fraîche et objet vérifié ; noter le digest actif et le dernier
   état sain. Préparer les deux images privées par digest et confirmer le pull
   authentifié sur Netcup. Vérifier les SHA-256 des scripts et SQL avant copie
   root-owned ; ne jamais utiliser une copie modifiable par `deoptops` en root.
2. Ouvrir une fenêtre d'indisponibilité DeOpt seulement. Arrêter le backend
   par Coolify, conserver PostgreSQL et le volume. Exécuter le préparateur de
   rôles avec le compte administrateur existant. Il ne change ni HBA ni schéma.
3. Dans la **même ressource Coolify**, passer au Compose socket candidate et
   à l'image candidate par digest, conserver le même projet Compose et le
   même volume PGDATA. Vérifier le rendu Raw, les chemins des trois secrets,
   l'absence de ports et de réseaux DB, puis déployer. Le backend doit rester
   non prêt tant que le schéma est à 71 ; son redémarrage ne migre pas.
4. Vérifier les connexions Docker réelles. Exécuter le migrateur une fois via
   `operator-migrate-least-privilege-v3.sh IMAGE_DIGEST SOURCE_COMMIT`. Vérifier
   72 migrations, SCRAM, droits, `/health`, `/ready`, auth wallet, refus anonyme,
   historique d'un autre wallet refusé, port PostgreSQL non publié, proxy hors
   socket, volumes conservés. Ne jamais faire de migration descendante.
5. Installer la nouvelle paire de scripts backup avec empreintes vérifiées,
   puis exécuter une sauvegarde avec le rôle backup et vérifier l'objet R2,
   les journaux et le timer. Confirmer que les autres projets et Coolify restent
   accessibles. Puis seulement ouvrir la route HTTPS prévue, sous une revue
   distincte de l'authentification et du routage ; le trading reste désactivé.

**Retour arrière :** avant 0072, restaurer le profil Compose et le digest
précédents en conservant le volume. Après 0072, conserver le profil socket et
passer au digest de l'image de repli : `/ready` doit rester 200 et les routes
wallet 403. Si PostgreSQL/socket lui-même échoue après 0072, arrêter l'API
DeOpt et réparer sous contrôle ; ne pas revenir à l'ancienne image incompatible
ni restaurer automatiquement la base active.

## Bloqueurs avant production

1. Approbation explicite du basculement PostgreSQL/Coolify et de 0072.
2. Commits de cette préparation non poussés ; nouvelles images privées et
   digests de registre non publiés ni tirés sur Netcup.
3. Rendu Coolify Raw et topologie après redéploiement sur Netcup non observés.
   La topologie Docker locale ne prouve pas la persistance après recréation du
   proxy partagé ; aucun redémarrage proxy n'est autorisé ici.
4. Sauvegarde R2 réelle avec le nouveau rôle backup et contrôles HTTPS hébergés
   à exécuter seulement après basculement.

Tests ciblés : `cargo build --offline --bin deopt-v2-backend`,
`cargo test --offline --lib auth::wallet_read`,
`cargo test --offline --lib read_only_release`,
`wallet_read_auth_pg` exécuté sur la copie réelle avec le rôle runtime et
`DEOPT_AUTH_TEST_SKIP_MIGRATIONS=1 DEOPT_AUTH_TEST_DISPOSABLE=1`,
`python3 -m unittest deploy.vps.test_least_privilege_compose deploy.vps.test_coolify_private_compose`,
`bash -n` des scripts, `rustfmt --edition 2021 --check` des fichiers
Rust modifiés, `docker build` des deux images locales, restaurations et
recréations Compose jetables. `cargo fmt --check` global échoue sur la dette
historique hors périmètre ; `cargo clippy --offline --bin deopt-v2-backend -- -D warnings`
échoue avec 197 diagnostics, aucun dans les fichiers Rust modifiés.
Aucun PASS global de formatage ou Clippy n'est revendiqué.
