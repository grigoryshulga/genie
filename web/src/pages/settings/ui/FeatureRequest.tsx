/** Visible pending capabilities reference their tracked follow-up. */
export function FeatureRequest({ id, title, children }: { id: string; title: string; children: React.ReactNode }) {
  return (
    <section className="st-card" aria-label={title}>
      <div className="st-card-head">
        <h3>{title}</h3>
        <p>{children}</p>
      </div>
      <div className="st-card-body">
        <span className="pill">Запланировано · Feature Request {id}</span>
      </div>
    </section>
  );
}
