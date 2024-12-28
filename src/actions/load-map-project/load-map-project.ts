import { readTextFile } from '@tauri-apps/plugin-fs';
import { loadAssets } from '^actions/load-map-project/load-assets/load-assets';
import { AppState } from '^src/state/app-state.ts';
import { Perf } from '^utils/perf/perf.ts';
import { effect } from '^utils/reactive/effect.ts';


export async function loadMapProject(projectFilePath: string) {
	const perf = Perf('loadMapProject');

	const mapProject = parseMapProject(await readTextFile(projectFilePath));
	AppState.mapProject.set(mapProject);

	perf();
}


function parseMapProject(mapProject: string) {
	// TODO: add validation
	return JSON.parse<MapProject>(mapProject);
}


effect([AppState.mapProject], function () {
	const mapProject = AppState.mapProject.value;
	if (mapProject) {
		loadAssets(mapProject);
	}
});
