import { SettingsFile } from '^storage/perma-storage/settings-file.ts';
import { fileExists } from '^utils/fs/file-exists.ts';


export async function updateMaxPath(path: string): Promise<void> {
	if (await doesAllFilesExists(path)) {
		SettingsFile.set({ max: { path } });
	} else {
		console.error('Missing files, is this the correct path MAX?');
		return;
	}
}


async function doesAllFilesExists(path: string): Promise<boolean> {
	return (await Promise.all([
		'MAX.RES',
		'CRATER_1.WRL',
		'CRATER_2.WRL',
		'CRATER_3.WRL',
		'CRATER_4.WRL',
		'CRATER_5.WRL',
		'CRATER_6.WRL',
		'DESERT_1.WRL',
		'DESERT_2.WRL',
		'DESERT_3.WRL',
		'DESERT_4.WRL',
		'DESERT_5.WRL',
		'DESERT_6.WRL',
		'GREEN_1.WRL',
		'GREEN_2.WRL',
		'GREEN_3.WRL',
		'GREEN_4.WRL',
		'GREEN_5.WRL',
		'GREEN_6.WRL',
		'SNOW_1.WRL',
		'SNOW_2.WRL',
		'SNOW_3.WRL',
		'SNOW_4.WRL',
		'SNOW_5.WRL',
		'SNOW_6.WRL',
	].map(function (file) {
		return fileExists(path + '/' + file);
	}))).every(function (exists) {
		return exists;
	});
}
