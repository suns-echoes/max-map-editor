import { createServer, IncomingMessage } from 'http';
import { readFile, readFileSync, writeFileSync } from 'fs';
import { writeFile } from 'fs';


const hostname = '127.0.0.1';
const port = 3101;


function readRequestBody(req: IncomingMessage): Promise<string> {
	return new Promise((resolve, reject) => {
		let body = '';
		req.on('data', chunk => {
			body += chunk.toString(); // convert Buffer to string
		});
		req.on('end', () => {
			resolve(body);
		});
		req.on('error', (err) => {
			reject(err);
		});
	});
}


const server = createServer(async (req, res) => {

	if (req.method === 'GET') {
		if (req.url === '/') {
			req.url = '/index.html';
		}

		const filePath = req.url!.substring(1);
		const ext = filePath.split('.').pop();

		let contentType = 'text/plain';
		if (ext === 'css') {
			contentType = 'text/css';
		} else if (ext === 'js') {
			contentType = 'application/javascript';
		} else if (ext === 'json') {
			contentType = 'application/json';
		} else if (ext === 'html') {
			contentType = 'text/html';
		}

		readFile(filePath, (err, data) => {
			if (err) {
				res.statusCode = 404;
				res.setHeader('Content-Type', 'text/plain');
				res.end('Not Found');
			} else {
				res.statusCode = 200;
				res.setHeader('Content-Type', contentType);
				res.end(data);
			}
		});
	}

	else if (req.method === 'POST') {
		const url = req.url;
		const body = await readRequestBody(req);

		if (false) {
			res.statusCode = 200;
			res.setHeader('Content-Type', 'application/json');
			// res.end(JSON.stringify({ message: 'Data received', data: body }));
			res.end('1');
		}

		else {
			res.statusCode = 404;
			res.setHeader('Content-Type', 'text/plain');
			res.end('Not Found');
		}
	}

	else {
		res.statusCode = 405;
		res.setHeader('Content-Type', 'text/plain');
		res.end('Method Not Allowed');
	}

});


server.listen(port, hostname, () => {
	console.log(`Server running at http://${hostname}:${port}/`);
});
