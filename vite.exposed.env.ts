// import { getName, getTauriVersion, getVersion } from '@tauri-apps/api/app';


export const exposedENV: Record<string, any> = {
	// app_name: await getName(),
	// app_version: await getVersion(),
	// tauri_version: await getTauriVersion(),

	// Update:
	// ./src-tauri/tauri.conf.json:package.version
	// ./src-tauri/Cargo.toml:version
	build_version: process.env.npm_package_version,

};
