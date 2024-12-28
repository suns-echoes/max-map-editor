import { SettingsFile } from '^storage/perma-storage/settings-file.ts';


await SettingsFile.sync();

console.info('Settings:', SettingsFile.getAll());

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
