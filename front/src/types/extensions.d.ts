interface Window {
	_templates: Map<string, HTMLTemplateElement>,
}

interface Document {
	createElement<T>(tagName: string, options?: ElementCreationOptions): T;
	getElementById<T>(elementId: string): T;
}

interface DocumentFragment {
	getElementById<T>(elementId: string): T;
}

interface JSON {
	parse<T>(text: string, reviver?: (this: any, key: string, value: any) => any): T;
}
