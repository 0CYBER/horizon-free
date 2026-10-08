package dev.horizonlab.budget;

public final class TerrainBudgetTest {
    private static void check(boolean value, String reason) { if (!value) throw new AssertionError(reason); }
    public static void main(String[] args) {
        TerrainBudget budget = new TerrainBudget(0.5);
        for (int i = 0; i < 100; i++) budget.update(0.5, 40, 16.667, 0.5, 0.3);
        check(budget.ratio() == 0.05, "Sustained pressure reaches a nonzero floor");
        for (int i = 0; i < 2; i++) budget.update(0.5, 10, 16.667, 0, 0.3);
        check(budget.ratio() == 0.05, "Recovery requires three healthy windows");
        budget.update(0.5, 10, 16.667, 0, 0.3);
        check(Math.abs(budget.ratio() - 0.075) < 1e-9, "Recovery is gradual");
        for (int i = 0; i < 1000; i++) budget.update(0.5, 10, 16.667, 0, 0.3);
        check(budget.ratio() == 0.5, "Recovery never exceeds the user's ceiling");
        check(budget.update(0.02, 40, 16.667, 0.5, 0.3) == 0.02, "Respect a user ceiling below our floor");
        budget = new TerrainBudget(0.5);
        check(budget.update(0.5, 10, 16.667, 0, 0.9) < 0.5, "Heap pressure reduces work");
        budget = new TerrainBudget(0.5);
        check(budget.update(0.5, 33.3, 1000.0 / 30, 0, 0.3) == 0.5, "Respect a 30 FPS cap");
        budget = new TerrainBudget(0.5);
        for (int i = 0; i < 100; i++) {
            budget.update(0.5, 40, 16.667, 0.5, 0.3);
            budget.update(0.5, 10, 16.667, 0, 0.3);
        }
        check(budget.ratio() == 0.05, "Alternating pressure cannot trigger rapid recovery");
        try { budget.update(0.5, Double.NaN, 16, 0, 0); throw new AssertionError("NaN accepted"); }
        catch (IllegalArgumentException expected) { }
        System.out.println("Terrain budget tests passed");
    }
}
