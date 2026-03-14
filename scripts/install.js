#!/usr/bin/env node
const { existsSync, chmodSync } = require('fs');
const path = require('path');

const projectRoot = path.join(__dirname, '..');
const platform = process.platform;
const arch = process.arch;
const isGlobalInstall =
  process.env.npm_config_global === 'true' || process.env.npm_config_global === '1';

// Map Node.js platform/arch to our binary naming convention
const getBinaryName = () => {
  const platformMap = {
    'darwin': 'darwin',
    'linux': 'linux',
    'win32': 'win32'
  };

  const archMap = {
    'arm64': 'arm64',
    'x64': 'x64'
  };

  const mappedPlatform = platformMap[platform];
  const mappedArch = archMap[arch];

  if (!mappedPlatform || !mappedArch) {
    console.error(`Unsupported platform: ${platform}-${arch}`);
    console.error('Supported platforms: macOS (arm64/x64), Linux (x64), Windows (x64)');
    process.exit(1);
  }

  const ext = platform === 'win32' ? '.exe' : '';
  return `learnchain-${mappedPlatform}-${mappedArch}${ext}`;
};

const binaryName = getBinaryName();
const binaryPath = path.join(projectRoot, 'dist', binaryName);

// Verify the pre-compiled binary exists
if (!existsSync(binaryPath)) {
  console.error(`Pre-compiled binary not found: ${binaryName}`);
  console.error(`Expected at: ${binaryPath}`);
  console.error('\nThis might mean:');
  console.error('1. Your platform is not supported');
  console.error('2. The package was not built correctly');
  console.error(`\nSupported: darwin-arm64, darwin-x64, linux-x64, win32-x64`);
  console.error(`Your platform: ${platform}-${arch}`);
  process.exit(1);
}

// Make binary executable (Unix systems)
if (platform !== 'win32') {
  try {
    chmodSync(binaryPath, 0o755);
  } catch (err) {
    console.warn(`Warning: Could not make binary executable: ${err.message}`);
  }
}

const commandPrefix = isGlobalInstall ? 'learnchain' : 'npx learnchain';
const startCommand = isGlobalInstall ? 'learnchain' : 'npx learnchain';

console.log(`✓ learnchain installed successfully for ${platform}-${arch}`);
console.log('');
if (isGlobalInstall) {
  console.log('LearnChain is ready.');
} else {
  console.log('LearnChain can be started from this project with npx.');
}
console.log(`Start the app: ${startCommand}`);
console.log(`Help: ${commandPrefix} --help`);
console.log(`Configure OpenAI: ${commandPrefix} config set openai-key <your-key>`);
console.log(`Install the Codex skill: ${commandPrefix} skill install codex`);
console.log('Learn more: https://learnchain.co');
