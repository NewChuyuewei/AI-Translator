const VERSION_PATTERN = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(alpha|beta|rc)\.(0|[1-9]\d*))?$/;

export function parseVersion(version) {
  const match = version?.match(VERSION_PATTERN);
  if (!match) {
    throw new Error('Version must be MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-(alpha|beta|rc).NUMBER');
  }

  const major = Number(match[1]);
  const minor = Number(match[2]);
  const patch = Number(match[3]);
  const channel = match[4] || null;
  const sequence = match[5] === undefined ? null : Number(match[5]);
  if (minor > 99 || patch > 999) throw new Error('Minor must be <= 99 and patch must be <= 999');

  let stage = 99;
  if (channel) {
    const limits = { alpha: 19, beta: 19, rc: 48 };
    if (sequence > limits[channel]) throw new Error(`${channel} sequence must be <= ${limits[channel]}`);
    stage = { alpha: 10, beta: 30, rc: 50 }[channel] + sequence;
  }

  const versionCode = major * 10_000_000 + minor * 100_000 + patch * 100 + stage;
  if (!Number.isSafeInteger(versionCode) || versionCode > 2_100_000_000) {
    throw new Error('Version is too large for Android versionCode');
  }

  return {
    version,
    baseVersion: `${major}.${minor}.${patch}`,
    fileVersion: `${major}.${minor}.${patch}.0`,
    versionCode,
    prerelease: channel !== null,
    channel,
    sequence,
  };
}
