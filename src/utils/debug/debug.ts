import { sleep } from '^utils/flow-control/sleep.ts';


export async function printDebugInfo(message: string) {
	const pre = document.createElement('pre') as HTMLParagraphElement;
	pre.textContent = message;
	(document.getElementsByClassName('debug-info')[0] as HTMLElement).appendChild(pre);
	return sleep(500);
}
