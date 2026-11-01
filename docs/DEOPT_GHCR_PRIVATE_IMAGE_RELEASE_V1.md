# DeOpt — image backend privée GHCR

État du 2026-11-01 UTC. Aucun package GHCR ni service DeOpt n'a été publié ou déployé dans ce jalon.

## Décision de confidentialité

La connexion GitHub en lecture seule confirme que `coriom/deopt-v2-backend` est **public** et que le compte connecté possède les droits administrateur sur ce dépôt. La CLI `gh` locale signale un jeton invalide ; l'existence et la visibilité d'un éventuel package GHCR privé préexistant ne sont donc pas vérifiées. Ne pas interpréter une absence dans une recherche anonyme comme une preuve d'absence du package.

[GitHub indique](https://docs.github.com/en/packages/managing-github-packages-using-github-actions-workflows/publishing-and-installing-a-package-with-github-actions) qu'un package créé par un workflow avec `GITHUB_TOKEN` hérite par défaut de la visibilité et des permissions du dépôt. Une première publication depuis le dépôt **public** ne répond pas à l'exigence « package privé garanti ». Le workflow manuel reste désormais bloqué tant que ce dépôt est public, même si un package privé existe, afin d'éviter l'accès potentiel des forks. Aucun lancement n'a eu lieu.

Deux chemins possibles pour lever ce blocage, chacun à approuver séparément :

1. Rendre le dépôt source privé, puis revérifier sa visibilité avant une première exécution du workflow ; ou
2. Précréer le package GHCR en privé via un mécanisme contrôlé ; préparer les futures publications Actions depuis un dépôt privé distinct avec droits explicitement revus.

Ne pas changer la visibilité du dépôt ni créer un package au moyen d'un token transmis dans le chat. Vérifier aussi le budget GitHub Actions applicable au compte avant le premier run ; la [facturation GHCR actuelle](https://docs.github.com/en/billing/concepts/product-billing/github-packages) ne remplace pas ce contrôle des minutes Actions.

## Workflow préparé

`.github/workflows/publish-private-ghcr.yml` est uniquement `workflow_dispatch`, limité à `main`, au dépôt canonique et à `contents:read` / `packages:write`. Il teste la confidentialité avec `deploy/vps/ghcr-private-preflight.sh`, exécute les contrôles de release, construit `linux/amd64` et pousse uniquement `ghcr.io/coriom/deopt-v2-backend:sha-<commit>`. Il ajoute les labels OCI `source` et `revision`, une provenance BuildKit minimale et capture le digest de manifeste du registre ; un `docker pull` authentifié par digest et des contrôles des labels suivent la publication. Aucun tag `latest`, aucun secret de build et aucun déploiement automatique. Les workflows sur push existants ont été relus : ils exécutent des tests CI, sans publication ni déploiement.

Le Dockerfile ne copie que `Cargo.toml`, `Cargo.lock`, `src`, `migrations`, `resources` et l'entrypoint. `.dockerignore` exclut les fichiers `.env*`, tests, docs et métadonnées Git. Trois fichiers d'exemple `.env*.example` sont suivis à la racine, mais exclus du contexte de build ; un scan des chemins réellement copiés ne trouve aucun fichier `.env`, `.pem`, `.key`, `.p12` ou `.dmp`. Cette vérification bornée ne prouve pas qu'aucune constante sensible n'existe dans le code compilé ; aucun secret n'est passé au build dans ce workflow.

Validation locale : 35 tests ciblés PASS, syntaxe Bash et YAML du workflow PASS. Construction Docker `linux/amd64` depuis `0572870e8114f49fd1a57bd0d3e7e6cd02a84aed` PASS, Dockerfile SHA-256 `0b051d09fa4b1e3ec60e710aa78bb8ae096cd56e4d38a6c7d9ab700420fb029f`, image locale `sha256:693d9953d74b74fd097c3f4a240a904cf692193fe7918adc3c8d12fef0bd4835`. Un conteneur jetable sans réseau a confirmé UID 1001, le binaire et les migrations, sans fichier de type clé ou dump dans `/app` et `/usr/local/bin`. Cette empreinte est un identifiant **local**, pas un digest de registre. Aucune exécution du workflow GitHub ni téléchargement GHCR n'a été testée.

## Coolify, sans déploiement

Après publication privée **vérifiée**, dans Coolify `Servers > localhost > Registries`, utiliser le preset GHCR et un compte ayant accès au package avec un PAT classic limité à `read:packages`. [Coolify documente](https://coolify.io/docs/applications/builds/docker-registries) que l'authentification est enregistrée par Docker sur le serveur, avec identifiants encodés et non chiffrés : limiter strictement les permissions du fichier et du token. Ne jamais placer le token dans Compose, Git, le chat ou les rapports.

La future variable `DEOPT_BACKEND_IMAGE` doit prendre la référence `ghcr.io/coriom/deopt-v2-backend@sha256:<digest-de-registre>`. Le digest local antérieur n'est pas utilisable comme digest GHCR. `DeOpt/staging` n'a pas été créé et aucun conteneur DeOpt n'a démarré.

## Portes restantes

Publication GHCR : **BLOCKED** par la visibilité publique du dépôt et l'absence de preuve d'un package privé préexistant. Déploiement privé VPS : toujours **BLOCKED** par le Compose réellement généré par Coolify, le secret PostgreSQL hôte, la sauvegarde hors VPS avec restauration, le digest GHCR et l'autorisation distincte de release. La publication 8080 du proxy Coolify reste un durcissement séparé avec revue et retour arrière. Aucun port, frontend, contrat ni état économique n'a été modifié.
