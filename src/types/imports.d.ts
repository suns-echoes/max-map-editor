declare module "*.html" {
	const template: HTMLTemplateElement;
	export default template;
}

declare module "*.style" {
	const content: HTMLStyleElement;
	export default content;
}

declare module "*.vs" {
	const content: string;
	export default content;
}

declare module "*.fs" {
	const content: string;
	export default content;
}
