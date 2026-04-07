#!/bin/sh
# Simple POSIX conformance test

echo "Test 1: Basic echo"
echo hello

echo "Test 2: Variable assignment"
x=42
echo $x

echo "Test 3: Command substitution"
y=$(echo test)
echo $y

echo "Test 4: If statement"
if true; then
    echo "if works"
fi

echo "Test 5: For loop"
for i in 1 2 3; do
    echo "item: $i"
done

echo "Test 6: Case statement"
case "abc" in
    abc) echo "case matches" ;;
    *) echo "case fails" ;;
esac

echo "Test 7: Functions"
myfunc() {
    echo "function works"
}
myfunc

echo "Test 8: Exit code"
false || echo "false failed as expected"
true && echo "true succeeded as expected"

echo "All tests completed!"