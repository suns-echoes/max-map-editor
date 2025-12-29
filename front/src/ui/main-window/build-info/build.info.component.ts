import { getAppVersion } from '^lib/info/info.ts';
import { Div } from '^lib/reactive/html-node.elements.ts';


export function BuildInfo() {
	return (
		Div('build-info').style({
			position: 'absolute',
			zIndex: '9999',
			bottom: '10px',
			right: '10px',
			color: 'white',
			background: 'rgba(0, 0, 0, 0.5)',
			padding: '5px',
		}).text(getAppVersion())
	);
}
