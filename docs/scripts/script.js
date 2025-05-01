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

document.querySelectorAll('img[data-lazy-src]').forEach(function(img) {
	if (img.getAttribute('src')) return;

	const imgXSS = /[:<>]/;

	const loadImage = function() {
		const src = img.getAttribute('data-lazy-src');
		if (imgXSS.test(src)) return;
		img.src = encodeURI(src);
		img.removeAttribute('data-lazy-src');
		img.removeEventListener('mouseenter', loadImage);
		img.removeEventListener('touchstart', loadImage);
		img.removeEventListener('focus', loadImage);
	};

	if ('IntersectionObserver' in window) {
		const observer = new IntersectionObserver(function(entries, observer) {
			entries.forEach(function(entry) {
				if (entry.isIntersecting) {
					loadImage();
					observer.unobserve(img);
				}
			});
		});
		observer.observe(img);
	} else {
		img.addEventListener('mouseenter', loadImage);
		img.addEventListener('touchstart', loadImage);
		img.addEventListener('focus', loadImage);
	}
});
