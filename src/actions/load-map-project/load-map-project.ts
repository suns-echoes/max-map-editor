import { readTextFile } from '^tauri-apps/plugin-fs.ts';
import { loadAssets } from '^actions/load-map-project/load-assets/load-assets.ts';
import { AppState } from '^state/app-state.ts';
import { Perf } from '^utils/perf/perf.ts';
import { effect } from '^utils/reactive/effect.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';


export async function loadMapProject(projectFilePath: string) {
	printDebugInfo('loadMapProject');

	const perf = Perf('loadMapProject');

	const mapFile = await readTextFile(projectFilePath);
	const mapProject = parseMapProject(mapFile);
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
