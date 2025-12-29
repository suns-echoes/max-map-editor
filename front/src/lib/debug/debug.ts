import { sleep } from '^lib/flow-control/sleep.ts';


export async function printDebugInfo(message: string) {
	// const pre = document.createElement('pre') as HTMLParagraphElement;
	// pre.textContent = message;
	// (document.getElementsByClassName('debug-info')[0] as HTMLElement).appendChild(pre);
	console.info(message);
	return sleep(50);
}
