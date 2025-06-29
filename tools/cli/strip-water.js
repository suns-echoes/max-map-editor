import { readFileSync, writeFileSync } from 'fs';

console.log('Stripping water from tiles.data.json');
console.log('Place this file in the directory with tiles.data.json');

const json = JSON.parse(readFileSync('./tiles.data.json', 'utf8'));

let r = 0;

Object.keys(json).forEach((key) => {
	if (key[1] === 'S') {
		json[key] = json[key].replace(/../g, (match) => {
			const pixel = parseInt(match, 16);
			if (pixel >= 96 && pixel <= 109) {
				r++;
				return '00';
			}
			return match;
		});
	}
})

writeFileSync('./tiles.data.json', JSON.stringify(json, null, '\t'));

console.log('Removed', r, 'water pixels');
