document.querySelectorAll('img[data-preview="true"]').forEach(function (image) {
	image.addEventListener('click', function (event) {
		event.preventDefault();

		const container = document.createElement('div');
		container.className = 'image-preview-container';

		const panel = document.createElement('div');
		panel.className = 'log-panel';
		container.appendChild(panel);

		if (image.title) {
			const title = document.createElement('div');
			title.className = 'label';
			title.textContent = image.title;
			panel.appendChild(title);
		}

		const grid = document.createElement('div');
		grid.className = 'grid';
		panel.appendChild(grid);

		const bigImgWrapper = document.createElement('div');
		grid.appendChild(bigImgWrapper);

		const bigImage = document.createElement('img');
		bigImage.src = image.src.replace('_small.', '.');
		bigImgWrapper.appendChild(bigImage);

		document.body.appendChild(container);

		container.addEventListener('click', function (event) {
			event.preventDefault();
			container.remove();
		}, { once: true });
	})
});
