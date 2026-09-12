# TODO — Investigate Cloudflare Pages as static host (alt/addition to Netlify)

Goal: decide whether Cloudflare Pages is a better home for the Speck frontend
(thin tester now, full MVP-2b app later) than Netlify. Decision doc, not a deploy.

## Questions to answer
- Free tier limits: builds/month, bandwidth, requests. (CF Pages is generous;
  confirm current numbers.)
- Static publish of `web-tester/` with SPA fallback:
  - CF Pages uses a `_redirects` file (`/*  /index.html  200`) instead of
    netlify.toml. Confirm syntax and that it ships in the publish dir.
- Does anything block outbound `wss://` from the browser? (No — wss is a
  client-side connection to Railway; the static host is irrelevant to it.)
- Custom domain + automatic HTTPS: compare setup vs Netlify.
- Build integration for full MVP-2b (Vite): build command `npm run build`,
  output dir (`dist`). CF Pages Git integration vs `wrangler pages deploy`.
- Wrangler CLI (`npx wrangler pages deploy web-tester`) for CLI-first deploys.

## Comparison axes vs Netlify
| Axis                | Netlify            | Cloudflare Pages     |
|---------------------|--------------------|----------------------|
| SPA fallback config | netlify.toml       | `_redirects` file    |
| CLI deploy          | netlify-cli deploy | wrangler pages deploy|
| Free bandwidth      | (fill in)          | (fill in)            |
| Edge network        | good               | very large (CF edge) |
| Build minutes       | (fill in)          | (fill in)            |

## Recommendation placeholder
- For a single static file the choice is near-irrelevant; pick by existing
  account / DNS. Revisit seriously when building MVP-2b (real Vite build).
