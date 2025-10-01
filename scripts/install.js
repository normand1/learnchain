#!/usr/bin/env node
const { spawnSync } = require('child_process');
const { existsSync, mkdirSync, copyFileSync, chmodSync, writeFileSync } = require('fs');
const path = require('path');

const projectRoot = path.join(__dirname, '..');

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    stdio: 'inherit'
  });

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

run('cargo', ['build', '--release']);

const platform = process.platform;
const arch = process.arch;
const binaryName = platform === 'win32' ? 'learnchain.exe' : 'learnchain';
const sourcePath = path.join(projectRoot, 'target', 'release', binaryName);

if (!existsSync(sourcePath)) {
  console.error(`learnchain binary not found at ${sourcePath}. Did the build succeed?`);
  process.exit(1);
}

const distDir = path.join(projectRoot, 'dist');
mkdirSync(distDir, { recursive: true });

const packagedBinaryName = `learnchain-${platform}-${arch}${platform === 'win32' ? '.exe' : ''}`;
const binaryDest = path.join(distDir, packagedBinaryName);
copyFileSync(sourcePath, binaryDest);
chmodSync(binaryDest, 0o755);

const launcherPath = path.join(distDir, 'learnchain.js');
const launcher = `#!/usr/bin/env node\n` +
  `const { spawn } = require('child_process');\n` +
  `const path = require('path');\n` +
  `const fs = require('fs');\n` +
  `const binaryName = ${JSON.stringify(packagedBinaryName)};\n` +
  `const binPath = path.join(__dirname, binaryName);\n` +
  `if (!fs.existsSync(binPath)) {\n` +
  `  console.error('learnchain binary not found for this platform.');\n` +
  `  process.exit(1);\n` +
  `}\n` +
  `const child = spawn(binPath, process.argv.slice(2), { stdio: 'inherit' });\n` +
  `child.on('close', (code) => process.exit(code ?? 0));\n`;
writeFileSync(launcherPath, launcher, { mode: 0o755 });
chmodSync(launcherPath, 0o755);

console.log(`learnchain binary packaged for ${platform}/${arch}`);

const envKey = process.env.OPENAI_API_KEY;
if (envKey && envKey.trim().length > 0) {
  console.log('Configuring OpenAI API key from environment...');
  run('node', [launcherPath, '--set-openai-key', envKey.trim()]);
} else {
  console.log('Tip: run `npx learnchain --set-openai-key <your-key>` or use the Config view to add your OpenAI API key.');
}
