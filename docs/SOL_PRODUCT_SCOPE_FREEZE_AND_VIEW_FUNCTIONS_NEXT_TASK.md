# Next-task prompt: SOL-PRODUCT-SCOPE-FREEZE-AND-VIEW-FUNCTIONS

Copy/paste this prompt verbatim to initiate the M-P1 milestone.

---

```
Workspace root is ~/DEOPT.

Execute SOL-PRODUCT-SCOPE-FREEZE-AND-VIEW-FUNCTIONS only.

This is a Solidity product-scope freeze + view-function additions milestone.
External audit is deferred until the platform is product-complete; see
~/DEOPT/deopt-v2-backend/docs/PRODUCT_READINESS_ROADMAP.md.

Do not deploy.
Do not broadcast.
Do not send transactions on any chain.
Do not create Safe transactions.
Do not create AWS resources.
Do not create KMS keys.
Do not edit `.env`.
Do not expose secrets.
Do not transfer ownership / guardian / proposer / executor on any deployed
contract.
Do not withdraw fees.
Do not allocate rebate budget.
Do not move funds.
Do not modify any behavioural surface (state-mutating function) of any
already-deployed contract; this milestone is VIEW-FUNCTION-ADDITIONS-ONLY.

Strategic context:

* DeOpt V2 sol contracts are largely mature (~69 production files;
  comprehensive test suite).
* Frontend trading UI does not yet exist; the UI requires a stable view-
  function surface to wire against.
* Backend trading APIs need stable contract ABIs to consolidate against.
* No mainnet activity is planned in this milestone.

Goal:
Freeze the Solidity ABI for the V2 product MVP and add only the minimal
view functions required by the frontend trading interface per
`PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md §2.2 + §2.3 + §2.4`. No
behavioural change to existing state-mutating functions. No storage layout
change. Additive ABI only.

Required Phase A — inspect:
Inspect `~/DEOPT/deopt-v2-sol/src/` and confirm presence (with file +
line refs) of each required view function:

* `OptionProductRegistry`: enumerable product list view + per-product
  detail view; `OptionSeriesCreated` event if absent.
* `MarginEngine`: `optionSeries(seriesId)` view + per-account position
  view aggregated by series + free-collateral view per account; if not
  already aggregated, add lens view in
  `~/DEOPT/deopt-v2-sol/src/lens/MarginEngineLens.sol`.
* `OracleRouter`: `price(underlying)` view (likely present); expiry-
  aware view if absent.
* `CollateralVault`: `balanceOf(account, token)` (present); free-
  collateral aggregated view from `RiskModule` if absent — add via lens.
* `FeesManagerV2`: `previewFee(quote)` trade-preview view for UI.
* `RiskModule`: `accountSummary(account)` returning (equity, IM, MM,
  freeCollateral) aggregated view; add via lens if absent.

For each: list whether present + signature. If missing, plan an additive
view function (lens preferred to avoid touching mainnet-deployable bytecode).

Required Phase B — additions (additive only):
Add lens contracts under `~/DEOPT/deopt-v2-sol/src/lens/` only. Allowed:

* new view functions on existing lens contracts;
* new lens contracts that read from existing core contracts via
  read-only interfaces.

Forbidden in this phase:

* modifying any function in `src/collateral/`, `src/matching/`,
  `src/margin/`, `src/perp/`, `src/risk/`, `src/fees/`, `src/oracle/`,
  `src/gouvernance/`, `src/liquidation/`, `src/core/`, `src/yield/`,
  `src/OptionProductRegistry.sol`;
* introducing new storage slots in any of the above modules;
* changing visibility / mutability of any existing function;
* modifying `script/*.sol` deploy scripts (separate milestone if rewire is needed);
* modifying mainnet manifest templates.

If an additive function MUST be added directly to a core contract (e.g.
a view that needs internal accessor), STOP and surface the question; do
not introduce it unilaterally.

Required Phase C — events:
If `OptionSeriesCreated`, `PositionSettled`, or any unified UI event tap
is missing per `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md §2.3`,
propose the event signature (no code change in this phase). Document the
proposed signature in the result doc. Operator approves before any sol
event-emit code change is landed in a follow-on milestone.

Required Phase D — tests:
For every new view function added in Phase B:

* unit test under `~/DEOPT/deopt-v2-sol/test/unit/lens/` verifying the
  expected return shape against a fresh deployment;
* a single scenario test under `~/DEOPT/deopt-v2-sol/test/scenario/`
  that exercises the view through the full UI-anchored flow (create
  series → quote → trade → position → exercise → settle) and asserts the
  view returns consistent data at each step.

No fork tests in this milestone.

Required Phase E — ABI freeze:
Generate the per-contract ABI snapshot from `forge build`:

* enumerate every public + external function with its selector;
* enumerate every event with its selector;
* enumerate every immutable + storage variable visible via auto-getter;
* compare against the prior known ABI snapshot (if present); confirm
  the diff is ADDITIVE-ONLY (no removed selector, no changed selector,
  no changed event signature).

Produce a single artefact at
`~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/`:

* `<contract>.abi.json` per in-scope production contract;
* `selectors.txt` listing every public selector with its source-file ref.

Required Phase F — storage layout pin:
For every in-scope contract: run `forge inspect <contract> storageLayout`;
diff against the prior known layout (if present); fail if any slot is
moved.

Required Phase G — freeze tag:
After Phases A-F land green: tag the repo `v2-product-freeze-rc1`. Do
NOT push the tag remotely in this milestone (operator pushes after
review).

Required Phase H — result doc:
Create `~/DEOPT/deopt-v2-sol/docs/SOL_PRODUCT_SCOPE_FREEZE_RESULT.md`.
Include:

* contracts frozen (list + commit hash);
* view-function additions (per file: function signature + selector +
  test reference);
* event additions PROPOSED (no code change; documented signature);
* ABI diff: ADDITIVE-ONLY count;
* storage-layout diff: zero slots moved;
* test additions: list + line counts;
* product-MVP lifecycle coverage table: each step (create / quote /
  fill / position / exercise / close / settle) maps to one view + one
  scenario test.

Required Phase I — validation:
Run only non-invasive validations:

* `forge fmt --check`;
* `forge build`;
* `forge test --no-match-path 'test/fork/*'`;
* `forge test --match-path 'test/scenario/*'`;
* storage-layout diff verifier (see Phase F);
* ABI diff verifier (see Phase E).

If any test fails: stop and report why. Do NOT mask the failure.

Required Phase J — RUN_STATE:
Update `~/DEOPT/deopt-v2-backend/RUN_STATE.md` (per existing convention).
Add one concise closure paragraph at the top:

* contracts frozen at tag `v2-product-freeze-rc1`;
* view functions added (count + file refs);
* events proposed (count; not yet emitted);
* test additions (count);
* ABI diff = additive only;
* storage layout diff = zero slots moved;
* next milestone routing.

No secrets.

Forbidden:

* no mainnet tx;
* no Sepolia tx;
* no live broadcast;
* no Safe tx;
* no governance mutation;
* no ownership transfer;
* no guardian mutation;
* no Timelock mutation;
* no fee withdrawal;
* no rebate allocation;
* no fund movement;
* no `.env` edit;
* no AWS resource creation;
* no KMS key creation;
* no deployment;
* no canary;
* no private key / admin token / RPC secret / DATABASE_URL / API key
  output;
* no AWS credentials;
* no real AWS account IDs;
* no real KMS key IDs;
* no real KMS ARNs;
* no guessed mainnet executor address;
* no private custody roster disclosure;
* no production signer address guess;
* no invented mainnet deployed contract addresses;
* no claim that audit has started;
* no claim that platform is audited;
* no behavioural change to existing state-mutating functions;
* no storage slot relocation;
* no event signature change;
* no public selector change.

Hard stops:

* stop if a task would require a real transaction;
* stop if a task would require a Safe transaction;
* stop if a task would require AWS resource creation;
* stop if a task would require editing `.env`;
* stop if a task would require revealing a secret;
* stop if an additive function MUST go into a core contract (not lens);
* stop if any storage slot would shift;
* stop if any existing public selector would change;
* stop if any existing event signature would change.

Return final report grouped by:
workspace,
repos inspected,
current sol product readiness gap,
view-function additions,
events proposed,
ABI freeze artefact,
storage layout diff,
tests added,
result doc,
RUN_STATE update,
files changed,
validations,
blockers,
next milestone recommendation.
```

---

**End of next-task prompt.**
