#!/usr/bin/env node
const { spawnSync } = require('node:child_process');
const result = spawnSync(process.env.CCSW_TEST_HELPER, process.argv.slice(2), { stdio: 'inherit', shell: false });
if (result.error) { console.error(result.error.message); process.exit(1); }
process.exit(result.status ?? 1);
