/* Browser acceptance for rows ingested from a genuine local Anvil receipt. */
const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');
const requireFrontend = createRequire(path.resolve(__dirname, '../../../deopt-v2-frontend/package.json'));
const { chromium } = requireFrontend('@playwright/test');

async function main() {
  const fixture = JSON.parse(fs.readFileSync('/tmp/deopt_anvil_multifill_actual.json'));
  const [buyer] = fixture.fixture.buyers;
  const [sellerA, sellerB] = fixture.fixture.sellers;
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const feeRequests = [];
  page.on('response', response => {
    if (response.url().includes('/perps/canonical-fees')) feeRequests.push({url:response.url(),status:response.status()});
  });
  await page.addInitScript(({account}) => {
    const provider = {
      request: async ({method}) => {
        if (method === 'eth_chainId') return '0x14a34';
        if (method === 'eth_accounts' || method === 'eth_requestAccounts') return account ? [account] : [];
        throw new Error(`Local read-only wallet does not support ${method}`);
      },
      on: () => {}, removeListener: () => {},
    };
    Object.defineProperty(window, 'ethereum', {value: provider, configurable: true});
  }, {account: buyer});
  const base = 'http://127.0.0.1:3001';
  try {
    await page.goto(`${base}/perps`);
    await page.getByTestId('wallet-connect-button').click();
    await page.getByTestId('widget-perps-book-feed-tab-fees').click();
    await page.getByTestId('perps-fee-row').first().waitFor();
    if (await page.getByTestId('perps-fee-row').count() !== 2) throw new Error('buyer must have exactly two real receipt rows');
    const buyerText = await page.getByTestId('perps-fees-history').innerText();
    if (!buyerText.includes('0.000000 mUSDC') || !buyerText.includes('Subaccount settlement attribution is unverified')) throw new Error('buyer fee display mismatch');
    await page.reload();
    await page.getByTestId('perps-fees-disconnected').waitFor();
    await page.getByTestId('wallet-connect-button').click();
    await page.getByTestId('perps-fee-row').first().waitFor();
    if (await page.getByTestId('perps-fee-row').count() !== 2) throw new Error('reload lost genuine rows');
    // The browser can only request wallet-scoped public chain records.
    for (const [seller, expected] of [[sellerA,'0.474660 mUSDC'],[sellerB,'0.949320 mUSDC']]) {
      await context.clearCookies();
      const sellerPage = await context.newPage();
      await sellerPage.addInitScript(({address}) => {
        Object.defineProperty(window, 'ethereum', {value:{
          request: async ({method}) => method === 'eth_chainId' ? '0x14a34' :
            method === 'eth_accounts' || method === 'eth_requestAccounts' ? [address] : Promise.reject(new Error('read-only wallet')),
          on:()=>{}, removeListener:()=>{},
        },configurable:true});
      }, {address:seller});
      sellerPage.on('pageerror', error => errors.push(error.message));
      await sellerPage.goto(`${base}/perps`);
      await sellerPage.getByTestId('wallet-connect-button').click();
      await sellerPage.getByTestId('widget-perps-book-feed-tab-fees').click();
      await sellerPage.getByTestId('perps-fee-row').first().waitFor();
      if (await sellerPage.getByTestId('perps-fee-row').count() !== 1) throw new Error('seller wallet row count mismatch');
      const text = await sellerPage.getByTestId('perps-fees-history').innerText();
      if (!text.includes(expected)) throw new Error(`seller fee mismatch: expected ${expected}`);
      await sellerPage.close();
    }
    if (errors.length) throw new Error(`frontend client exceptions: ${JSON.stringify(errors)}`);
    if (!feeRequests.length || feeRequests.some(item => item.status !== 200 || !item.url.startsWith('http://127.0.0.1:4010/'))) throw new Error(`fee requests did not reach local backend: ${JSON.stringify(feeRequests)}`);
    process.stdout.write(`CONNECTED_BROWSER_PASS buyer_rows=2 seller_rows=1+1 backend_requests=${feeRequests.length} client_exceptions=0\n`);
  } finally {
    await browser.close();
  }
}
main().catch(error => {console.error(error);process.exitCode=1});
