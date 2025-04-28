import { relative, join } from 'path';

const fileToDebug = process.argv[1];

const relativePath = relative(
  join(import.meta.dirname, '..'),
  fileToDebug,
).split('/');

const fileName = relativePath.pop();
const fileDir = relativePath.join('/');

console.log(`\x1b[2J\x1b[H\x1b[36mDEBUG: ${fileDir}/\x1b[1m${fileName}\x1b[0m`);
console.log('');
console.log('\x1b[34m▰▰▰ START ▰▰▰\x1b[0m');

process.on('exit', () => {
  console.log('\x1b[34m▰▰▰ END ▰▰▰\n\n\x1b[0m');
});
