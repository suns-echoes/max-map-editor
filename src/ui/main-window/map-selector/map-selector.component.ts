import { resolveTextResource } from '^tauri-apps/api/path.ts';
import { loadMapProject } from '^actions/load-map-project/load-map-project.ts';
import { printDebugInfo } from '^utils/debug/debug.ts';
import { Div } from '^utils/reactive/html-node.elements.ts';
import { SimpleButton } from '^src/ui/components/buttons/simple-button.component.ts';

import style from './map-selector.module.css';
import { HTMLNode } from '^utils/reactive/html-node.class.ts';
import { sleep } from '^utils/flow-control/sleep.ts';


export function MapSelector() {
	printDebugInfo('UI::MapSelector');

	const buttons: HTMLNode<HTMLButtonElement>[] = [];

	const mapSelector = (
		Div('map-selector').class(style.mapSelector).nodes([
			buttons[0] = SimpleButton().text('Iron Cross'),
			buttons[1] = SimpleButton().text('Splatterscape'),
			buttons[2] = SimpleButton().text('Peak-a-boo'),
			buttons[3] = SimpleButton().text('Valentine\'s Planet'),
			buttons[4] = SimpleButton().text('Three Rings'),
			buttons[5] = SimpleButton().text('Great divide'),
		])
	);

	buttons.forEach(function (button, index) {
		button.addEventListener('click', async () => {
			buttons.forEach(function (button) {
				button.disable();
			});
			await loadMapProject(await resolveTextResource(`resources/maps/CRATER_${index + 1}.json`));
			await sleep(100);
			buttons.forEach(function (button) {
				button.enable();
			});
		});
	});

	return mapSelector;
}
