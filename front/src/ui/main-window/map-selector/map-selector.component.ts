import { resolveTextResource } from '^tauri-apps/api/path.ts';
import { AppState } from '^state/app-state.ts';
import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { printDebugInfo } from '^lib/debug/debug.ts';
import { animationFrame } from '^lib/flow-control/animation-frame.ts';
import { Effect } from '^reactive/effect.ts';
import { ReactiveNode } from '^reactive/reactive-node.class.ts';
import { Br, Div } from '^reactive/reactive-node.elements.ts';
import { SimpleButton } from '^src/ui/components/buttons/simple-button.component.ts';

import style from './map-selector.module.css';


function waitForMapSize(): Promise<Size> {
	return new Promise((resolve) => {
		const effect = new Effect(() => {
			const size = AppState.mapSize.value;
			if (size.width > 0 && size.height > 0) {
				effect.dispose();
				resolve(size);
			}
		}, { strong: true });
	});
}


const maps = [
	['CRATER_1', 'Iron Cross'],
	['CRATER_2', 'Splatterscape'],
	['CRATER_3', 'Peak-a-boo'],
	['CRATER_4', 'Valentine\'s Planet'],
	['CRATER_5', 'Three Rings'],
	['CRATER_6', 'Great divide'],
	null,
	['DESERT_1', 'Freckles'],
	['DESERT_2', 'Sandspit'],
	['DESERT_3', 'Great Circle'],
	['DESERT_4', 'Long Passage'],
	['DESERT_5', 'Flash Point'],
	['DESERT_6', 'Bottleneck'],
	null,
	['GREEN_1', 'New Luzon'],
	['GREEN_2', 'Middle Sea'],
	['GREEN_3', 'High Impact'],
	['GREEN_4', 'Sanctuary'],
	['GREEN_5', 'Islandia'],
	['GREEN_6', 'Hammerhead'],
	null,
	['SNOW_1', 'Snowcrab'],
	['SNOW_2', 'Frigia'],
	['SNOW_3', 'Ice Berg'],
	['SNOW_4', 'The Cooler'],
	['SNOW_5', 'Ultima Thule'],
	['SNOW_6', 'Long Floes'],
]


export function MapSelector() {
	printDebugInfo('UI::MapSelector');

	const buttons: [fileName: string, button: ReactiveNode<HTMLButtonElement>][] = [];

	const mapSelector = (
		Div('map-selector').class(style.mapSelector).nodes(
			maps.map((map) => {
				if (map === null) return Br();
				const button = SimpleButton().text(map[1]);
				buttons.push([map[0], button]);
				return button;
			})
		)
	);

	function disableAllButtons() {
		buttons.forEach(function ([, button]) {
			button.disable();
		});
	}

	function enableAllButtons() {
		buttons.forEach(function ([, button]) {
			button.enable();
		});
	}

	buttons.forEach(function ([mapFile, button]) {
		button.on('click', async () => {
			disableAllButtons();

			await animationFrame();

			AppState.reset();
			await loadMapProject(await resolveTextResource(`resources/maps/${mapFile}.json`));
			await waitForMapSize();

			await animationFrame();

			enableAllButtons();
		});
	});

	return mapSelector;
}
