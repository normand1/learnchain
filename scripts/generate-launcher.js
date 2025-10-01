#!/usr/bin/env node
const { writeFileSync, chmodSync, existsSync, readdirSync } = require('fs');
const path = require('path');

const projectRoot = path.join(__dirname, '..');
const distDir = path.join(projectRoot, 'dist');

// Check if dist directory exists and has binaries
if (!existsSync(distDir)) {
  console.error('Error: dist/ directory not found. Run the build workflow first.');
  process.exit(1);
}

const binaries = readdirSync(distDir).filter(f => f.startsWith('learnchain-') && !f.endsWith('.js'));

if (binaries.length === 0) {
  console.error('Error: No pre-compiled binaries found in dist/');
  console.error('Expected files like: learnchain-darwin-arm64, learnchain-linux-x64, etc.');
  process.exit(1);
}

console.log(`Found ${binaries.length} pre-compiled binaries:`);
binaries.forEach(b => console.log(`  - ${b}`));

// Generate the launcher script
const launcherPath = path.join(distDir, 'learnchain.js');
const launcher = `#!/usr/bin/env node
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const platform = process.platform;
const arch = process.arch;

// Map Node.js platform/arch to binary names
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
    console.error(\`Unsupported platform: \${platform}-\${arch}\`);
    console.error('Supported: macOS (arm64/x64), Linux (x64), Windows (x64)');
    process.exit(1);
  }

  const ext = platform === 'win32' ? '.exe' : '';
  return \`learnchain-\${mappedPlatform}-\${mappedArch}\${ext}\`;
};

const binaryName = getBinaryName();
const binPath = path.join(__dirname, binaryName);

if (!fs.existsSync(binPath)) {
  console.error(\`Binary not found for your platform: \${binaryName}\`);
  console.error(\`Expected at: \${binPath}\`);
  console.error(\`\\nYour platform: \${platform}-\${arch}\`);
  console.error('Supported platforms: darwin-arm64, darwin-x64, linux-x64, win32-x64');
  process.exit(1);
}

// Spawn the binary with all arguments
const child = spawn(binPath, process.argv.slice(2), { stdio: 'inherit' });
child.on('close', (code) => process.exit(code ?? 0));
`;

writeFileSync(launcherPath, launcher, { mode: 0o755 });
chmodSync(launcherPath, 0o755);

console.log(`\n✓ Generated launcher: ${launcherPath}`);
console.log('✓ Ready for npm publish');
