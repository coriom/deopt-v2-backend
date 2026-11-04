# DEOPT_OPTIONS_END_TO_END_BETA_V1

Date de vérification : 2026-11-04 UTC. Périmètre : Base Sepolia 84532 et tests locaux isolés. Aucune transaction publique, migration de production, activation de worker, publication ou push.

## État on-chain épinglé

RPC public Base Sepolia, bloc **54 414 188**, timestamp **1793804706**. Les lectures ci-dessous ne préjugent pas de l'état d'un bloc ultérieur.

| Élément | Résultat |
| --- | --- |
| OptionProductRegistry | `0x3d52b033fab00ed6104dd3bc0a715f8648344eca`, 2 séries |
| OptionMatchingEngine canonique | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6`, `paused=false`, pointe vers le Registry et le MarginEngine ci-dessous |
| MarginEngine canonique | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`, `paused=false`, `tradingPaused=false`, `settlementPaused=false`, `matchingEngine` = moteur canonique, `useFeesManagerV2=true` |
| Dépendances MarginEngine | RiskModule `0xc0f019005a25524a34f2ee8839dcdcc50715dd7b`, OracleRouter `0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581`, CollateralVault `0x00340c360353a5ab784c5bc5c44322a6af0625d3`, InsuranceFund `0x009f38440f058d095b61e0e2ee7fabdf05be7500` |
| Vault | `marginEngine` = moteur canonique ; `isAuthorizedEngine(moteur canonique)=true` |
| Série 0 | ID `24145907678156652148089862289363692212069910767044828147380657249455352740183` ; Call européen, active, expiry 1893456000, strike 3000 × 1e8, underlying `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02`, règlement mUSDC `0x6eAe407f5640B006faC9965182e238582A3B412E` |
| Série 1 | ID `61659809894441151155362259623021372286927233602859377750709900502125356171572` ; Call européen, inactive, même expiry, strike 90000 × 1e8, underlying `0x9D871aC7595E8Da271E866608E5145252047967c` |
| Exécuteur | `isExecutor` = false pour Timelock, Safe et ancien déployeur vérifiés. L'allowlist complète n'a pas été établie : **UNKNOWN**. Aucun envoi ne doit être préparé en supposant qu'un de ces comptes est autorisé. |

Les deux feeds sont configurés actifs avec `maxDelay=1500s` et `maxDeviationBps=100`. Les quatre sources retournent des prix datés de 4, 9, 125 et 426 secondes avant le bloc ; les deux écarts sont **166 bps** et **124 bps**. `OracleRouter.getPriceSafe` retourne `(0,0,false)` pour les deux paires. La cause observée est la divergence au-delà de 100 bps, pas un horodatage périmé. Toute modification d'oracle, de seuil ou de source exige une décision économique distincte et une nouvelle lecture.

| Paire | Source primaire : prix 1e8 / âge | Source secondaire : prix 1e8 / âge | Écart | Verdict |
| --- | --- | --- | --- | --- |
| Série 0 | `0x09BCd126FEB91e13007b2C69644c12AB56260473` : 243815000000 / 9 s | `0x1a6302eDC9e9DC5EFaE8Dd5e931089ac6Dc5322b` : 239814700144 / 125 s | 166 bps | `ok=false` |
| Série 1 | `0x78BFe7a71bD7C39d53d71b8A688BfB4AFB216867` : 8007280000000 / 4 s | `0x690821211CA653b966DB07deD385cFa4c547a6c2` : 7908869148801 / 426 s | 124 bps | `ok=false` |

## Parcours et écarts

| Porte | Verdict | Preuve et limite |
| --- | --- | --- |
| OPTIONS_CONTRACT_READINESS | PASS local / BLOCKED Base Sepolia | Signatures EIP-712 des deux contreparties, nonce et metadata sont vérifiés par `OptionMatchingEngine.executeTrade`. `MarginEngine` tient les positions, transfère prime/collatéral et règle après expiration. Les prix OracleRouter sont actuellement indisponibles. |
| OPTIONS_DEPLOYMENT_STATUS | PASS pour la pile legacy | Registry, OME et MarginEngine ont répondu aux lectures épinglées. Les contrats Hybrid V2 expérimentaux ne sont pas établis comme pile canonique déployée. |
| OPTIONS_PRICING_AND_ORACLES | BLOCKED | Le prix d'exécution est la prime **signée par acheteur et vendeur** ; un aperçu/Greek UI reste indicatif. Les deux feeds retournent `ok=false`. Aucun prix exécutable ne peut être déduit d'une maquette ou d'un aperçu. |
| OPTIONS_RISK_AND_COLLATERAL | PASS local / BLOCKED hosted | Tests de payoff et collatéral Call/Put ; Vault/RiskModule/InsuranceFund câblés au moteur déployé. Les paramètres de risque et capacités réelles des wallets doivent être revalidés avant une transaction testnet. |
| OPTIONS_ORDER_LIFECYCLE | PASS contrat local / BLOCKED intégration hébergée | Le nouveau test traverse signature → OME → MarginEngine → position → expiration → settlement et rejette le rejeu. Il n'existe pas de fonction d'exercice anticipé dans la pile legacy ; `isEuropean` est une métadonnée. Le règlement requiert un prix finalisé après expiry. |
| OPTIONS_BACKEND_INTEGRATION | PASS local partiel / BLOCKED hosted | 101 tests API, 152 tests Options et 35 tests PostgreSQL du product surface passent. La production a `OPTIONS_ENABLED=false`, indexer et workers économiques désactivés ; le middleware read-only refuse les routes Options. Les lectures privées Options ne bénéficient pas encore du contrôle de session wallet déployé pour `canonical-fees`. |
| OPTIONS_FRONTEND_INTEGRATION | PASS local partiel / BLOCKED hosted | Le terminal Options existe ; deux tests Playwright de fallback passent. La passerelle Vercel publique autorise santé et Perps seulement ; les requêtes Options sont refusées. Les écritures sont aussi bloquées par `isPublicTradingEnabled` en production. Les données et Greeks des fixtures Playwright ne sont pas des données de marché. |
| OPTIONS_E2E_TESTS | PASS couches ciblées / BLOCKED bout-en-bout | 20 tests OME, 8 tests de scénario dont Call signé + Put, 35 tests PostgreSQL et 2 tests navigateur. Il manque un test unique navigateur → Rust → PostgreSQL → contrats sur Anvil/fork avec indexation et redémarrage ; ne pas présenter ces couches séparées comme sa réussite. |
| OPTIONS_SECURITY_INVARIANTS | PASS local partiel / BLOCKED hosted | Rejeu de signature, transaction invalide, double règlement, conservation du collatéral et isolation des sous-comptes sont testés localement. Authentification session Options, autorisation par wallet, rate limits et exposition publique doivent être éprouvés avant activation. Aucun signer de production ne doit être ajouté à Netcup. |
| BASE_SEPOLIA_READINESS | BLOCKED | Seul un Call est actif, aucun Put enregistré, oracles refusés, exécuteur effectif inconnu, backend/indexer désactivés, API Options non exposée. |

### Modifications locales

- Backend : `GET /options/products` retourne `SOURCE_UNAVAILABLE` (503) quand Options est désactivé, au lieu d'un catalogue vide ou d'une erreur 500 trompeuse. Aucun changement du profil hébergé.
- Frontend : un refus HTTP sans JSON produit une erreur d'indisponibilité lisible, sans interpréter le refus comme une panne de parsing.
- Contrats : tests seulement ; scénario Put européen, et scénario Call signé sur le vrai OME avec transfert de prime, anti-rejeu et règlement. Aucun Solidity de production modifié.

### Tests exécutés

- `forge test --match-path test/unit/matching/OptionMatchingEngine.t.sol` : 20 PASS.
- `forge test --match-path test/scenario/options/OptionSettlementFlow.t.sol` : 8 PASS après les ajouts.
- `cargo test --lib api::trading::tests::` : 101 PASS.
- `cargo test --test options_tests` : 152 PASS, 6 ignored préexistants.
- `cargo test --test options_backend_product_surface_pg_integration -- --test-threads=1` : 35 PASS sur PostgreSQL 16 jetable lié uniquement à 127.0.0.1 ; conteneur retiré ensuite.
- Playwright local : nouveau refus Options sans JSON PASS ; fallback existant PASS. Premier lancement Chromium dans le sandbox : SIGTRAP, relancé avec succès hors sandbox local.
- `npm run typecheck`, ESLint des deux fichiers touchés, `rustfmt --check src/api/trading.rs`, `forge fmt --check` sur le scénario : PASS.
- `cargo fmt --check` global : FAIL sur formatage historique hors du fichier modifié. Aucun formatage global ni contournement.

## Séquence minimale vers une première transaction Options Base Sepolia

1. Décider et valider la source de prix et la tolérance de divergence ; obtenir `getPriceSafe(...).ok=true` au bloc frais pour l'underlying retenu, et vérifier les paramètres de marge/fees.
2. Choisir le Call actif pour le premier essai. La prise en charge Put sur Base Sepolia requiert la création/configuration d'une série Put par l'autorité compétente, avec autorisation distincte.
3. Vérifier l'allowlist `isExecutor` complète, le nonce, les soldes mUSDC déposés, les rôles de règlement et la capacité de pause. Définir un exécuteur testnet approuvé hors du VPS en lecture seule.
4. Sur infrastructure isolée, réussir un vrai parcours navigateur → backend → PostgreSQL → Anvil/fork → indexer → navigateur, avec signatures des deux wallets, Call/Put, redémarrage, reorg simulée et refus d'accès croisé. Ne pas activer la diffusion publique pour ce test.
5. Préparer une release séparée : indexation canonique, auth wallet pour toutes les lectures Options privées, routes publiques limitées, contrôle d'ordre/prime/risque, flags de trading, rollback et pause. Valider avant tout déploiement backend/frontend, migration ou activation de broadcaster.
6. Obtenir des autorisations distinctes pour les changements on-chain nécessaires, une transaction testnet bornée, puis une ouverture utilisateur. Aucun de ces actes n'est autorisé par ce jalon.

NEXT_ACTION : traiter la divergence des deux feeds Options par une décision oracle documentée, puis exécuter le test d'intégration isolé complet. Les décisions Perps V10 et pauses D4/D6 restent inchangées.
