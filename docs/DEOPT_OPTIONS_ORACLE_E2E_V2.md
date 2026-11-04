# DEOPT_OPTIONS_ORACLE_E2E_V2

Vérification locale du 4 novembre 2026 UTC. Périmètre : Base Sepolia 84532 en lecture seule, tests isolés. Aucune transaction publique, modification d'oracle, migration de production, activation économique, déploiement hébergé ou push.

## Oracles : preuve épinglée

La sonde reproductible est `deopt-v2-sol/scripts/options_oracle_preflight_v2.py`. Elle vérifie le chainId, lit le routeur, les deux adaptateurs, les réponses brutes Chainlink/Pyth et `getPriceSafe` au **même bloc**, contrôle les normalisations 1e8 et la cohérence du hash de bloc avant/après. Commande : `python3 scripts/options_oracle_preflight_v2.py --block 54427428` depuis le dépôt Solidity. Le RPC public par défaut est `https://sepolia.base.org`. Un RPC alternatif HTTPS peut être fourni par `DEOPT_READONLY_RPC_URL` sans apparaître dans l'output.

OracleRouter `0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581` ; quote mUSDC `0x6eAe407f5640B006faC9965182e238582A3B412E`. Les deux feeds sont actifs, avec `maxDelay=1500s`, délai global `1500s`, `maxDeviationBps=100`, `paused=false`, `readPaused=false` aux trois blocs. Chainlink répond en 8 décimales ; Pyth a `expo=-8`. Les deux prix normalisés sont donc identiques aux prix bruts indiqués. Les IDs Pyth configurés sont les IDs ETH/USD et BTC/USD documentés par [Pyth](https://docs.pyth.network/pro/use-prices-on-blockchain/evm) et [son API officielle](https://docs.pyth.network/api-reference/hermes/latest-price-updates). Les champs Chainlink proviennent de [`AggregatorV3Interface`](https://docs.chain.link/data-feeds/api-reference). L'adaptateur Pyth lit `conf`, mais ne lui applique aucun seuil : il utilise `price` et `publishTime`. Le heartbeat contractuel du feed Chainlink n'est pas exposé par cette interface ; seule la limite de fraîcheur du routeur a été vérifiée. La lecture directe de l'API Hermes a renvoyé HTTP 403 ; aucune comparaison numérique indépendante avec une API fournisseur n'est revendiquée.

| Bloc / hash | ETH Chainlink / Pyth (1e8), âges | BTC Chainlink / Pyth (1e8), âges | Écart ETH/BTC | `getPriceSafe` |
| --- | --- | --- | --- | --- |
| `54414188` / `0x4f09ad0b2b001ffb05f7879decf33251d6c364e2d0d59543daa6cb56ea48a0fd` | 243815000000 / 239814700144, 9 / 125 s | 8007280000000 / 7908869148801, 4 / 426 s | 166 / 124 bps | false / false |
| `54421424` / `0xe0dc9778c9a621c80304dbf26615e04b9f2e544398933d7175aa4bfc2fb0d1c9` | 243219440504 / 243167907196, 48 / 53 s | 8041355964821 / 8035705491211, 87 / 15 s | 2 / 7 bps | true / true |
| `54427428` / `0x4236d3fa0d12c17ca912ca5da0c0f4efdf776cb91bfb062c73c546f1894ef9e9` | 242201829411 / 242285497669, 62 / 38 s | 8044449000000 / 8035705491211, 24 / **1216** s | 3 / 10 bps | true / true |

Sources ETH : Chainlink adapter `0x09BCd126FEB91e13007b2C69644c12AB56260473`, aggregator `0x4aDC67696bA383F43DD60A9e78F2C97Fbbfc7cb1` (`ETH / USD`) ; Pyth adapter `0x1a6302eDC9e9DC5EFaE8Dd5e931089ac6Dc5322b`, contrat `0x5f52e4DBEA21f5b23523B6e20d50c29ae0a4EB83`, ID `0xff61491a931112ddf1bd8147cd1b641375f79f5825126d665480874634fd0ace`.

Sources BTC : Chainlink adapter `0x78BFe7a71bD7C39d53d71b8A688BfB4AFB216867`, aggregator `0x0FB99723Aee6f420beAD13e6bBB79b7E6F034298` (`BTC / USD`) ; Pyth adapter `0x690821211CA653b966DB07deD385cFa4c547a6c2`, même contrat Pyth, ID `0xe62df6c8b4a85fe1a67db44dc12de5db330f7ac66b72dc658afedf0f4a415b43`.

La cause démontrée du refus V1 est la **divergence entre deux prix valides et encore frais au sens du routeur**, au-delà de 100 bps. Les identifiants, cotations et décimales ne montrent pas d'erreur de configuration. La différence de temps de publication est une explication plausible, sans preuve qu'elle soit l'unique cause économique. Les mises à jour ultérieures ont ramené les écarts sous 100 bps sans changement on-chain. Le BTC Pyth était proche de sa limite de fraîcheur au dernier bloc. Si une source devient périmée, le routeur peut accepter la seule autre source ; une réponse `ok=true` n'est donc pas systématiquement une confirmation à deux sources. Aucune hausse de tolérance ni prix forcé n'est justifiée par ces observations.

## Intégration et sécurité

La pile historique `OptionMatchingEngine` exige deux signatures EIP-712 et `onlyExecutor`; le test contractuel Call/Put et le règlement passent. Le scénario V2 ajoute un Call signé sur le vrai `RiskModule`, avec collatéral, transfert de prime, positions et règlement ; seul l'oracle de ce test est synthétique et déterministe. Le banc Rust `scripts/e2e/run_e2e.py` vérifie des fills et frais **hors chaîne**, sans transaction Anvil. L'indexeur Options sait enregistrer des logs, mais aucune preuve V2 ne relie une exécution Anvil à PostgreSQL, à une position canonique et à l'affichage navigateur. `GET /accounts/:wallet/positions` calcule aujourd'hui ses positions depuis `OptionFill`, pas depuis `MarginEngine.positions`. Le fixture Playwright renvoie une forme de signature EIP-712 fictive. Il serait incorrect de réunir ces tests indépendants sous `FULL_E2E_INTEGRATION=PASS`.

Corrections locales V2 : les cinq routes `/accounts/:wallet/{positions,portfolio,balances,history,history/v2}` comparent l'URL à la session wallet côté Rust quand l'auth wallet est activée. Un test sur PostgreSQL 16 jetable vérifie 401 anonyme et 403 inter-wallet pour chacune. La passerelle Next `/api/deopt/options` n'autorise que les lectures catalogue et ces cinq routes ; les lectures privées transmettent le cookie de session au Rust via `Authorization` serveur, avec `Cache-Control: no-store, private`. Le test HTTP local a obtenu 403 pour `/admin/status`, 401 pour les lectures wallet anonymes, 503 pour le catalogue en l'absence d'upstream. Le profil `DEOPT_READ_ONLY_RELEASE` hébergé refuse toujours toute route Options et tout flag `OPTIONS_ENABLED=true`; aucun changement de production n'est effectué.

Le contrôle final fait échouer les cinq lectures privées avec 503 si l'auth wallet n'est pas activée, au lieu d'accepter une simple adresse dans l'URL. L'API marque les positions déduites des fills `partial` même si une marque oracle est disponible ; l'interface affiche explicitement que ces positions ne sont pas réconciliées avec `MarginEngine` et ne transforme plus une réponse partielle vide en « aucune position ouverte ». Les données restent hors chaîne et ne constituent pas une position canonique. Le mode actuel d'auth wallet exige par configuration le profil hébergé en lecture seule, alors que ce profil refuse les routes Options : ce couplage doit être résolu et testé avant publication des lectures Options.

## Verdicts

| Porte | Verdict et preuve |
| --- | --- |
| `ORACLE_ROOT_CAUSE` | **PASS** pour la cause immédiate : divergences 166/124 bps entre sources fraîches au bloc V1. Origine économique de ces différences **UNKNOWN**. |
| `ORACLE_FEED_VALIDITY` | **PASS on-chain** pour IDs, cotations, normalisation, âge, round Chainlink et réponse Pyth aux blocs cités. Heartbeat fournisseur et comparaison Hermes directe **BLOCKED** (HTTP 403). |
| `ORACLE_RESOLUTION` | **PASS ponctuel** aux deux blocs V2 ; aucune intervention on-chain requise pour obtenir `getPriceSafe=true` à ces blocs. Disponibilité continue **BLOCKED** sans surveillance fraîche. |
| `GET_PRICE_SAFE` | **PASS** : `(0,0,false)` V1 et prix normal V2 ; test du seuil exact 100 bps et de l'échec sur timestamps invalides. |
| `FULL_E2E_INTEGRATION` | **BLOCKED** : aucun test unique navigateur → Rust → PG → Anvil → indexation → navigateur. |
| `EIP712_ORDER_EXECUTION` | **PASS contrat local** (Call signé, `RiskModule` réel et oracle synthétique, replay rejeté dans le test V1) ; **BLOCKED intégré** avec Rust/browser/Anvil. |
| `POSTGRES_INDEXATION` | **BLOCKED intégré** : indexeur existant, aucun receipt Anvil réellement indexé dans ce jalon. |
| `FRONTEND_POSITION_DISPLAY` | **BLOCKED canonique** : positions actuelles dérivées des fills backend, maintenant marquées partielles ; fixture EIP-712 navigateur non authentique. |
| `EXPIRATION_SETTLEMENT` | **PASS contrat local** (9/9 scénarios, dont un avec `RiskModule` réel) ; **BLOCKED intégré** pour affichage et réconciliation. |
| `WALLET_DATA_ISOLATION` | **PASS local** pour les cinq routes avec auth active et PostgreSQL jetable ; la configuration hébergée garde Options indisponible. |
| `BASE_SEPOLIA_READINESS` | **BLOCKED** : exécuteur effectif non établi, aucun Put enregistré, indexation/positions canoniques et pipeline hébergé indisponibles. |

## Tests et suite minimale

- `python3 -m unittest discover -s scripts -p test_options_oracle_preflight_v2.py` : 6 PASS.
- `python3 scripts/options_oracle_preflight_v2.py --block 54414188`, `--block 54421424`, sans `--block` : lectures RPC PASS ; aucun envoi.
- `forge test --match-path test/scenario/options/OptionSettlementFlow.t.sol` : 9 PASS.
- `cargo test --lib api::trading::tests::wallet_account` : 3 PASS après correction fail-closed.
- `cargo test --lib api::trading::tests::account_positions_impl` : PASS, avec avertissement canonique.
- `cargo test --test wallet_read_auth_pg` sur PostgreSQL 16 jetable : 1 PASS, 401/403 pour cinq routes ; conteneur retiré.
- `cargo test --lib read_only_release::tests::rejects_every_http_mutation_method` : PASS, refus Options conservé.
- `npm run typecheck`, ESLint des fichiers frontend modifiés : PASS. Next local : 403 admin, 401 wallet anonyme, 503 catalogue sans upstream, `no-store, private` partout.
- `forge build` global lancé puis interrompu après plusieurs minutes sans résultat au-delà de « Compiling 115 files with Solc 0.8.30 » ; verdict global **INCONCLUSIVE**. Le test Foundry ciblé de 9 scénarios a passé avant cet essai.
- Les warnings Clippy/formatage historiques hors fichiers touchés ne sont pas présentés comme corrigés.

Pour le premier trade **Call** testnet : (1) revalider feed `getPriceSafe` et fraîcheur au bloc canonique ; (2) déterminer l'exécuteur réellement autorisé par `OptionMatchingEngine.isExecutor`, les rôles de règlement, soldes mUSDC et paramètres de risque ; (3) relier un vrai trade signé sur Anvil au Rust et à PG, puis projeter les positions depuis l'état canonique et les afficher dans Playwright sans signatures factices ; (4) tester reorg, redémarrage et double règlement dans cette chaîne ; (5) obtenir les autorisations distinctes pour tout changement on-chain requis, transaction testnet bornée, déploiement backend/frontend et ouverture utilisateur. Le Call actif n'exige pas une nouvelle série ; un Put exige une création/configuration de série approuvée. La pile legacy ne fournit pas d'exercice anticipé : le règlement suit expiration et prix finalisé.

`NEXT_ACTION` : implémenter le pont canonique trade Anvil → indexeur Rust → projection de position → navigateur avec signatures EIP-712 réelles, dans un profil local isolé. Aucune activation Base Sepolia tant que cette preuve et les rôles ne sont pas établis.
