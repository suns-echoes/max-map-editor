import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { isES2024Supported } from '^utils/debug/is-es2024-supported.ts';
import { isWebGL2Supported } from '^utils/debug/is-webgl2-supported.ts';
import { sleep } from '^utils/flow-control/sleep.ts';

import globalStyle from './styles/global.style';
import style from './styles/index.style';


document.head.appendChild(globalStyle);
document.head.appendChild(style);

console.log((import.meta as any).env);
console.log(window.__ENV__);


await printDebugInfo('M.A.X. Game Map Editor');
// await printDebugInfo('version: ' + window.__ENV__.build_version);
await printDebugInfo(isWebGL2Supported() ? 'WebGL 2.0 supported' : 'WebGL 2.0 not supported');
await printDebugInfo(isES2024Supported() ? 'ES2024 supported' : 'ES2024 not supported');
await printDebugInfo('loading settings...');

await SettingsFile.sync();

await printDebugInfo('settings synced');

console.info('Settings:', SettingsFile.getAll());

await printDebugInfo('should display setup? ' + SettingsFile.get('setup'));

await sleep(3000);

//
// SETUP
//
if (SettingsFile.get('setup')) {
	await(await import('^actions/setup-screen/show-setup-screen')).showSetupScreen();
	await(await import('^actions/main-window/save-main-window-params')).saveMainWindowParams();
}

//
// MAIN WINDOW
//
await (await import('^actions/main-window/show-main-window.ts')).showMainWindow();




// document.body.addEventListener('dblclick', async function () {
// 	console.log('Opening folder dialog...');
// 	const path = await openFolderDialog();

// 	if (path) {
// 		console.log('Selected path:', path);
// 		updateMaxPath(path);
// 	}
// });


// document.body.addEventListener('click', async function () {
// 	console.log('Opening file dialog...');
// 	const path = await openFileDialog();

// 	if (path) {
// 		getZipFileList(path);
// 	}
// });

// document.body.addEventListener('click', async function () {
// 	console.log('Opening file dialog...');
// 	const path = await openFileDialog();

// 	if (path) {
// 		console.log('Selected path:', await MD5(await readFile(path)));
// 	}
// });

// document.getElementById('setup')!.addEventListener('click', async function () {
// 	console.log('SETUP...');

// 	await setup();
// });
