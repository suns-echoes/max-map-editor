import { strict as assert } from 'assert';
import { Signal } from './signal.class';


describe('Signal', function (){
	it('should create an empty signal', function (){
		const signal = Signal.empty();
		assert.equal(signal.value, undefined);
	});

	it('should notify observers if empty signal dispatches', function (){
		const signal = Signal.empty();
		let called = false;
		signal.observers.add(function () { called = true; });
		signal.dispatch();
		assert.equal(called, true);
	});

	it('should create a comparator signal', function (){
		const source = new Signal(5);
		const comparatorSignal = Signal.comparator(source, 5);
		assert.equal(comparatorSignal.value, true);
		source.set(10);
		assert.equal(comparatorSignal.value, false);
	});

	it('should notify observers if comparator signal value changes', function () {
		const source = new Signal(5);
		const comparatorSignal = Signal.comparator(source, 5);
		let called = false;
		comparatorSignal.observers.add(function () { called = true; });
		source.set(10);
		assert.equal(called, true);
		assert.equal(comparatorSignal.value, false);
		source.set(5);
		assert.equal(comparatorSignal.value, true);
	});

	it('should create a signal with initial value', function (){
		const signal = new Signal(10);
		assert.equal(signal.value, 10);
	});

	it('should destroy the signal', function (){
		const signal = new Signal(10);
		signal.destroy();
		assert.equal(signal.value, undefined);
		assert.equal(signal.observers.size, 0);
	});

	it('should dispatch the signal', function (){
		const signal = new Signal(10);
		let called = false;
		signal.observers.add(function () { called = true; });
		signal.dispatch();
		assert.equal(called, true);
	});

	it('should set a new value and notify observers', function (){
		const signal = new Signal(10);
		let prevValue, newValue;
		signal.observers.add(function (prev, next) { prevValue = prev; newValue = next; });
		signal.set(20);
		assert.equal(signal.value, 20);
		assert.equal(prevValue, 10);
		assert.equal(newValue, 20);
	});

	it('should not notify observers if value is the same', function (){
		const signal = new Signal(10);
		let called = false;
		signal.observers.add(function () { called = true; });
		signal.set(10);
		assert.equal(called, false);
	});

	it('should use custom equality check', function (){
		const signal = new Signal(10, { equal(a, b) { return a % 2 === b % 2 } });
		let called = false;
		signal.observers.add(function () { called = true; });
		signal.set(12);
		assert.equal(called, false);
		signal.set(11);
		assert.equal(called, true);
	});
});
