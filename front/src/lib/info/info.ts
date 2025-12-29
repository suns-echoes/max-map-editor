import packageJson from '../../../../package.json' assert { type: 'json' };

type PackageJson = { version?: string };

export function getAppVersion(): string {
	return (packageJson as PackageJson).version || 'unknown';
}
