let lastZIndex = 0;

function makeDraggable(elements) {
    elements.forEach((element) => {
        let offsetX, offsetY;

        element.addEventListener('mousedown', (event) => {
            offsetX = event.clientX - element.getBoundingClientRect().left;
            offsetY = event.clientY - element.getBoundingClientRect().top;
            document.addEventListener('mousemove', onMouseMove);
            document.addEventListener('mouseup', onMouseUp);
            element.style.cursor = 'grabbing'; // Change cursor to indicate dragging
            if (event.button === 0) {
                // Main button: bring to front
                lastZIndex += 1;
                element.style.zIndex = lastZIndex;
            } else if (event.button === 2) {
                // Secondary button: bring to back
                element.style.zIndex = 0;
            }
        });

        function onMouseMove(event) {
            element.style.position = 'absolute';
            element.style.left = `${Math.floor((event.clientX - offsetX) / state.zoom) * state.zoom}px`;
            element.style.top = `${Math.floor((event.clientY - offsetY) / state.zoom) * state.zoom}px`;
        }

        function onMouseUp() {
            document.removeEventListener('mousemove', onMouseMove);
            document.removeEventListener('mouseup', onMouseUp);
            element.style.cursor = 'grab'; // Change cursor back to default
        }
    });
}