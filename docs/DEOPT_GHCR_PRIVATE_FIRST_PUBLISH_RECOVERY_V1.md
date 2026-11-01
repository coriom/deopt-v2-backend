# DeOpt — première publication privée GHCR

État du 2026-11-01 UTC. Le dépôt `coriom/deopt-v2-backend` reste public. L'opérateur voit « Get started with GitHub Packages » dans son compte, ce qui suggère l'absence de package visible mais ne prouve pas à lui seul son inexistence. La CLI `gh` disponible dans cette session a un jeton invalide ; le connecteur GitHub authentifié confirme le dépôt et ses droits, mais n'expose pas d'API Packages. **Package existant : NOT_VERIFIED_BY_AUTHENTICATED_API.**

## Procédure préparée

`deploy/vps/ghcr-private-first-publish.sh --publish-private` s'exécute **uniquement dans le terminal WSL de l'opérateur**. Il demande un PAT classic temporaire sans écho. Il vérifie le compte `coriom`, limite les scopes acceptés à `write:packages` et, si GitHub l'ajoute ou l'exige, `read:packages` ; il refuse `repo`, `delete:packages` et les autres scopes. L'[API Packages](https://docs.github.com/en/rest/packages/packages) exige `read:packages` pour lire l'inventaire : si ce droit manque, le script s'arrête sans push.

Le script vérifie la branche `main`, un worktree propre, l'égalité HEAD local/distant et l'inventaire complet des packages du compte. Il distingue `ABSENT`, `EXISTS_PRIVATE`, `EXISTS_PUBLIC`, `ACCESS_DENIED` et `UNKNOWN`. Un package préexistant, y compris privé, arrête **cette première publication** ; un package public n'est jamais modifié. Il recontrôle l'absence juste avant le push.

Pour une absence confirmée, le script construit `linux/amd64` depuis le commit contrôlé, sans provenance BuildKit ni label `org.opencontainers.image.source` vers le dépôt public. Il vérifie architecture, label de révision, absence du label de liaison, UID 1001 et fichiers d'exécution dans un conteneur local sans réseau. Il demande ensuite `PUBLISH_PRIVATE` dans le terminal avant un seul `docker push` vers le tag `sha-<commit>`. Docker utilise un `DOCKER_CONFIG` temporaire et protégé ; le token est transmis à `docker login --password-stdin`, jamais sur la ligne de commande. Le dossier et les identifiants sont supprimés à la sortie.

[GHCR documente](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry) qu'une première publication en ligne de commande est privée par défaut et n'est pas liée au dépôt par le seul nom. Le label OCI `source` peut [lier automatiquement un package](https://docs.github.com/en/packages/learn-github-packages/connecting-a-repository-to-a-package) : il est donc exclu ici. Le script revérifie immédiatement via API la visibilité **Private** et l'absence de lien au dépôt, exige l'échec de lecture anonyme avec erreur d'autorisation ou de manifeste masqué, puis télécharge par digest avec l'authentification temporaire et vérifie le label du commit.

Le tag contient le commit mais reste techniquement modifiable par un administrateur GHCR ; le digest retourné après push constitue l'identité de contenu à utiliser pour la future release.

## Accès Actions futur

Le workflow Actions du dépôt public est désormais bloqué à la fois par sa condition de job et par son préflight, y compris si le package GHCR est privé. **Aucun accès Actions au package n'est configuré.** [GitHub avertit](https://docs.github.com/en/packages/managing-github-packages-using-github-actions-workflows/publishing-and-installing-a-package-with-github-actions) que des forks d'un dépôt public autorisé peuvent, dans certains cas, accéder au package privé. Une future publication automatique nécessite une décision séparée, de préférence un dépôt privé de publication avec droits limités, sans duplication du code applicatif.

## Validation et limites

Les 42 tests ciblés passent, dont un parcours interactif complet avec faux PAT et faux registre : inventaire, confirmation, push, vérifications et suppression du `DOCKER_CONFIG` temporaire. La syntaxe Bash et YAML passe. Le build Docker local `--provenance=false` du commit `724d0408b946f1d544db93b5328135a3e2c99b57` passe en `linux/amd64` ; l'image locale `sha256:b99908e18b7a75489776dbd1547150408054c1a04f70e1422c2e42bbb9695d71` porte la révision attendue et aucun label `source`. Son conteneur jetable sans réseau confirme UID 1001, binaire et migrations. Cet ID local n'est **pas** un digest GHCR. Aucun PAT réel n'a été reçu par Codex, aucun push GHCR ni pull distant n'a été exécuté. La confidentialité effective, le digest de registre et le refus anonyme restent **NOT_VERIFIED** jusqu'à l'exécution opérateur.

La publication d'image n'autorise aucun déploiement. Coolify, Netcup, Vercel, les ports, le protocole et les états de trading ne changent pas. Les portes VPS antérieures (sauvegarde/restauration DeOpt, secret PostgreSQL, Compose généré, isolation réseau, autorisation de release) restent ouvertes.

## Intervention opérateur unique

Créer dans GitHub un PAT classic temporaire limité à `write:packages` et, si nécessaire pour l'API, `read:packages`, sans `repo`. Dans le terminal WSL avec Docker et `gh` disponibles, exécuter :

```bash
cd /home/corio/DEOPT/deopt-v2-backend
bash deploy/vps/ghcr-private-first-publish.sh --publish-private
```

Saisir le PAT uniquement au prompt masqué, puis `PUBLISH_PRIVATE` après lecture du commit et du tag. Ne transmettre ensuite que les lignes non sensibles `PACKAGE_VISIBILITY`, `ANONYMOUS_PULL`, `AUTHENTICATED_PULL`, `SOURCE_COMMIT` et `REGISTRY_DIGEST`. Révoquer le PAT temporaire après succès ou abandon. Si le script s'arrête, transmettre uniquement son code d'erreur, jamais le PAT ni les logs temporaires.
