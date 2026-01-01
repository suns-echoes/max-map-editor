#! /usr/bin/env node

//
// ▰▰▰ CLI HELP ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰
//

import fs from 'fs/promises';
import path from 'path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import { CliLog } from '../_common/node/cli-log.js';

const cliLog = new CliLog();
const appDir = process.cwd();
const execFileAsync = promisify(execFile);

if (process.argv[2] === '-h' || process.argv[2] === '--help') {
	cliLog.bgYellow().black()
		.ln('')
		.ln('')
		.ln(' ▟██▙  ██▙           ▟██▘      ▜█▙   ▟█▛')
		.ln('  ███▙ ███▙         ▟███▙       ▜█▙ ▟█▛')
		.ln('  ██▜█▙██▜█▙       ▟█▛ ▜█▙       🬸███🬴')
		.ln('  ██ ▜███ ▜█▙ ▜█▛ ▟███████▙ ▜█▛ ▟█▛ ▜█▙ ▜█▛')
		.ln('  ██  ▜██  ▜█▙ ▀ ▟█▛     ▜█▙ ▀ ▟█▛   ▜█▙ ▀')
		.ln('', true)
		.ln('')
		.bold().green().ln('Archive compressor (v 1.0.0)', true)
		.ln('')
		.ln('Compress directories using either LZ4HC or tar+gzip.')
		.ln('')
		.ln('Pre-requisites:')
		.ln('    lz4 command line tool must be installed and available in PATH for -lz4 option')
		.ln('    tar and gzip must be available in PATH for -tgz option')
		.ln('')
		.ln('Usage:')
		.ln('    ./compress.js input_dir output_file [options]')
		.ln('')
		.ln('Arguments:')
		.ln('    input_dir   - path to the directory you wish to archive')
		.ln('    output_file - destination archive file path')
		.ln('')
		.ln('Options:')
		.ln('    -h, --help     Show this help message')
		.ln('    -lz4           Create a tar stream piped through lz4 --best')
		.ln('    -tgz           Create a tar.gz archive using gzip -9')
		.ln('    -r, --relative  Resolve output_file relative to input_dir')
		.ln('')
		.ln('Example:')
		.ln(`    ./compress.js ./assets ./archives/assets.tar.lz4 -lz4`)
		.ln(`    ./compress.js ./assets ./archives/assets.tar.gz -tgz`)
		.ln('')
		.ln('')
		.ln('Path:')
		.green(appDir)
		.ln('')
		.reset().log();
	process.exit(0);
}

export async function compressFileToLZ4HC(inputDir, outputFile) {
	const sourceRoot = path.resolve(inputDir);
	const parentDir = path.dirname(sourceRoot);
	const baseName = path.basename(sourceRoot);
	const cmd = `tar -C "${parentDir}" -cf - "${baseName}" | lz4 -z -f --best > "${outputFile}"`;
	await execFileAsync('/bin/bash', ['-lc', cmd]);
	return outputFile;
}

export async function compressFileToTarGz(inputDir, outputFile) {
	const sourceRoot = path.resolve(inputDir);
	const parentDir = path.dirname(sourceRoot);
	const baseName = path.basename(sourceRoot);
	const cmd = `tar -C "${parentDir}" -I 'gzip -9' -cf "${outputFile}" "${baseName}"`;
	await execFileAsync('/bin/bash', ['-lc', cmd]);
	return outputFile;
}

const args = process.argv.slice(2);
const positionalArgs = [];
let useLz4 = false;
let useTgz = false;
let resolveRelativeToInput = false;

for (const arg of args) {
	if (arg === '-lz4') {
		useLz4 = true;
		continue;
	}
	if (arg === '-tgz') {
		useTgz = true;
		continue;
	}
	if (arg === '-r' || arg === '--relative') {
		resolveRelativeToInput = true;
		continue;
	}
	if (arg.startsWith('--relative=')) {
		cliLog.red().ln('Error: --relative option does not accept a value.').reset().log();
		process.exit(1);
	}
	positionalArgs.push(arg);
}

if (positionalArgs.length !== 2) {
	cliLog.red().ln('Error: Expected input_dir and output_file arguments.').reset().log();
	process.exit(1);
}

if (useLz4 && useTgz) {
	cliLog.red().ln('Error: Choose only one compression method (-lz4 or -tgz).').reset().log();
	process.exit(1);
}

if (!useLz4 && !useTgz) {
	cliLog.red().ln('Error: No compression method specified. Use -lz4 or -tgz.').reset().log();
	process.exit(1);
}

const [inputDirArg, outputFileArg] = positionalArgs;
const resolvedInputDir = path.isAbsolute(inputDirArg)
	? inputDirArg
	: path.resolve(appDir, inputDirArg);
const resolvedOutputFile = path.isAbsolute(outputFileArg)
	? outputFileArg
	: resolveRelativeToInput
		? path.resolve(path.dirname(resolvedInputDir), outputFileArg)
		: path.resolve(appDir, outputFileArg);
const compressor = useLz4 ? compressFileToLZ4HC : compressFileToTarGz;
const methodLabel = useLz4 ? 'tar|lz4 --best' : 'tar|gzip -9';

(async () => {
	try {
		const stats = await fs.stat(resolvedInputDir);
		if (!stats.isDirectory()) {
			throw new Error('Input path must be a directory.');
		}

		await fs.mkdir(path.dirname(resolvedOutputFile), { recursive: true });
		await compressor(resolvedInputDir, resolvedOutputFile);

		cliLog.green()
			.ln(`Compressed "${resolvedInputDir}" -> "${resolvedOutputFile}" using ${methodLabel}.`)
			.reset().log();
	} catch (err) {
		cliLog.red().ln(`Compression failed: ${err.message}`).reset().log();
		process.exit(1);
	}
})();

