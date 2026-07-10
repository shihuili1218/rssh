interface HomeRefreshDependencies<StaticData, DynamicData> {
  loadStatic: () => Promise<StaticData>;
  loadDynamic: () => Promise<DynamicData>;
  applyStatic: (data: StaticData) => void;
  applyDynamic: (data: DynamicData) => void;
}

export function createHomeRefresh<StaticData, DynamicData>(
  dependencies: HomeRefreshDependencies<StaticData, DynamicData>,
) {
  let generation = 0;

  return {
    async refresh(): Promise<void> {
      const current = ++generation;
      const staticData = await dependencies.loadStatic();
      if (current !== generation) return;
      dependencies.applyStatic(staticData);

      const dynamicData = await dependencies.loadDynamic();
      if (current !== generation) return;
      dependencies.applyDynamic(dynamicData);
    },
    cancel(): void {
      generation += 1;
    },
  };
}
