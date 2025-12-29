import { ErrorModal, type ErrorModalProps } from '^src/ui/components/modals/error-modal/error-modal.component';


export function showErrorModalAction(props: ErrorModalProps): LockPromise {
	return new Promise<void>(async function (resolve) {
		ErrorModal({
			title: props.title ?? 'Error',
			message: props.message ?? 'An unexpected error occurred.',
			onClose: function () {
				props.onClose?.();
				resolve();
			},
		});
	});
}
