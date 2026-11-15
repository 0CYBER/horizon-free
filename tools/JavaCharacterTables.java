// Generates and dumps the tables behind crates/horizon-core/src/java_character.rs from the public
// java.lang.Character API (isLetterOrDigit(char), digit(char, int)); no JDK implementation source is read.
//
//   java tools/JavaCharacterTables.java tables   prints LETTER_OR_DIGIT and DIGITS as Rust source
//   java tools/JavaCharacterTables.java dump     prints "unit letterOrDigit [digit for each of 39 radices]" for
//                                                every unit that is a letter or digit (the digits only when
//                                                some radix accepts it), for the daily test
//                                                java_character::tests::matches_java_character
//
// Tables in the repository were produced by:
//   openjdk version "25.0.5" 2026-10-20 LTS
//   OpenJDK Runtime Environment Temurin-25.0.5+8 (build 25.0.5+8-LTS)
//   OpenJDK 64-Bit Server VM Temurin-25.0.5+8 (build 25.0.5+8-LTS, mixed mode, sharing)
//   runtime: java.vendor "Eclipse Adoptium", java.runtime.version "25.0.5+8-LTS", Unicode 16 (Java 25)

public final class JavaCharacterTables {
    // Radices a caller can pass: 0 to 37 (both sides of the valid 2..=36) and the largest unsigned 32-bit value.
    static final int[] RADICES = new int[39];

    static {
        for (int radix = 0; radix <= 37; radix++) {
            RADICES[radix] = radix;
        }
        RADICES[38] = -1; // u32::MAX as a Java int
    }

    public static void main(String[] args) {
        switch (args.length == 1 ? args[0] : "") {
            case "tables" -> tables();
            case "dump" -> dump();
            default -> {
                System.err.println("usage: JavaCharacterTables tables|dump");
                System.exit(2);
            }
        }
    }

    static void tables() {
        System.out.println("const LETTER_OR_DIGIT: &[(u16, u16)] = &[");
        int start = -1;
        for (int unit = 0; unit <= 0x10000; unit++) {
            boolean in = unit <= 0xFFFF && Character.isLetterOrDigit((char) unit);
            if (in && start < 0) {
                start = unit;
            } else if (!in && start >= 0) {
                System.out.printf("    (0x%04X, 0x%04X),%n", start, unit - 1);
                start = -1;
            }
        }
        System.out.println("];");
        System.out.println();
        System.out.println("const DIGITS: &[(u16, u16, u8)] = &[");
        int first = 0;
        int previous = -1;
        for (int unit = 0; unit <= 0x10000; unit++) {
            int value = unit <= 0xFFFF ? Character.digit((char) unit, 36) : -1;
            boolean continues = start >= 0 && value >= 0 && value == previous + 1;
            if (start >= 0 && !continues) {
                System.out.printf("    (0x%04X, 0x%04X, %d),%n", start, unit - 1, first);
                start = -1;
            }
            if (value >= 0 && start < 0) {
                start = unit;
                first = value;
            }
            previous = value;
        }
        System.out.println("];");
    }

    static void dump() {
        for (int unit = 0; unit <= 0xFFFF; unit++) {
            char c = (char) unit;
            boolean letterOrDigit = Character.isLetterOrDigit(c);
            StringBuilder digits = new StringBuilder();
            boolean any = false;
            for (int radix : RADICES) {
                int value = Character.digit(c, radix);
                any |= value >= 0;
                digits.append(' ').append(value);
            }
            if (letterOrDigit || any) {
                System.out.println(unit + " " + letterOrDigit + (any ? digits : ""));
            }
        }
    }
}
